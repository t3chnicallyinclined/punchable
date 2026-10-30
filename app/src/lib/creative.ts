// Typed client for the creative routes in docs/API_CONTRACT.md. Bodies are the shapes in creative/CREATIVE.md.
//
// DEV ONLY fallback: while a contract route is not live yet (404/405/501 or no route), a __DEV__ build answers it
// from src/lib/devFixtures.ts (the creative layer's own demo outputs, copied by `npm run dev-fixtures`).
// Every such answer carries `__fixture: true`, and screens show the <FixtureBanner>. Production builds never
// fall back: the error surfaces as an error.
import { useCallback } from 'react';
import { ApiError, get, patch, post, useApi } from './api';

declare const __DEV__: boolean;
const DEV = typeof __DEV__ !== 'undefined' && __DEV__;

// ------------------------------------------------------------------ shapes
export type Source = 'his' | 'inferred' | 'proposed';
export type Ev = { node: string; quote: string; field?: string };
export type ConnItem = {
  id: string; joke_id: string; title: string; stage: string; score: number; why: string; source: Source; basis: string[];
  evidence: Ev[]; edge?: string; nights?: number; unit_id?: string; line?: string; personas?: string[]; topics?: string[]; shared?: string[]; direction?: string;
};
export type ConnGroupKey = 'next' | 'tags' | 'same_world' | 'callbacks' | 'same_shape' | 'contrasts';
export type ConnGroup = { key: ConnGroupKey; title: string; items: ConnItem[] };
export type TrendNight = {
  date: string; set: string; venue: string; label: string; laugh_s_per_min: number; vs_room: number | null; ai_rating: string | null;
  laugh_s: number; duration_s: number; time_to_first_laugh_s: number | null; seg_ids: string[];
};
export type Connections = {
  joke: { id: string; title: string; stage: string; stage_source: string; summary: string; devices: string[]; duration_s: number; duration_basis: string; rooms: number; nights: number };
  groups: ConnGroup[];
  stage_trend: {
    nights: TrendNight[]; trend: string | null; avg_laugh_s_per_min: number | null; avg_vs_room: number | null;
    best_night: (Partial<TrendNight> & { basis?: string }) | null; low_n: boolean; note?: string;
  };
  land_harder: { text: string; basis: string; source: Source; evidence: Ev[] }[];
  notes: string[];
};

export type Transition = {
  from: string; to: string; label: 'proven' | 'link' | 'jump'; why: string; via: { kind: string; name?: string; source?: string; edge?: string }[];
  needs_bridge: boolean; shared_topics: string[]; nights: { set: string; date: string; venue: string }[]; reverse_order_nights?: number; evidence: Ev[];
};
export type BoardBit = { joke_id: string; title: string; stage: string; duration_s: number; duration_basis: string; energy: number | null; heat: number; nights: number; devices: string[] };
export type Health = {
  board: string; title: string; runtime_s: number; runtime: string; target_s: number; bits: BoardBit[]; transitions: Transition[];
  energy_curve: { joke_id: string; energy: number | null; heat: number }[]; checks: { rule: string; status: 'ok' | 'warn' | 'info'; text: string; suggest?: any; pairs?: any }[];
  untaped: string[];
};
export type Board = {
  id: string; title: string; joke_ids: string[]; target_s: number; notes: string; source: Source; by: string; seed?: any;
  created_at: string; updated_at: string; deleted?: boolean;
};
export type BestNext = { joke_id: string; title: string; stage: string; score: number; why: string; source: Source; transition: string; energy: number | null; nights: number; evidence: Ev[] };
export type BoardView = { board: Board; health: Health; best_next: BestNext[]; setlist: { board: string; title: string; lines: string[]; runtime: string; text: string }; __fixture?: boolean };
/** One row of GET /api/boards (api-dev/API.md "Creative routes"): `archived` lists are left out unless `?archived=1`;
 *  `source` is "his" once he made or touched it, else "inferred" (a draft the app made); `by` is who created it
 *  (his id, `ai:seed`, `ai:demo`); `seed` names where a draft came from. The screen words come from rules/setListRules. */
export type BoardSeed = { from?: string | null; chunk?: string | number | null; minutes_est?: number | null };
export type BoardListItem = {
  id: string; title: string; n: number; runtime: string; runtime_s?: number; updated: string; target_s?: number; source?: Source; working_set?: string | null;
  archived?: boolean; by?: string | null; seed?: BoardSeed | null; names?: string[];
};
/** PATCH /api/boards/<id>: every key is checked before any is written; `unarchive` runs first, so it combines with a
 *  title; `archive` / `unarchive` are each other's undo and never change `source`; `delete` is terminal. */
export type BoardPatch = {
  title?: string; joke_ids?: string[]; target_s?: number; notes?: string; add?: { joke_id: string; at?: number }; remove?: { joke_id: string };
  move?: { from: number; to: number }; archive?: true; unarchive?: true; delete?: true;
};
export type Bridge = {
  from: string; to: string; from_title?: string; to_title?: string; transition?: string; draft: true; label: string;
  status: 'ok' | 'no_grounding' | 'provider_error' | 'queued'; text?: string; grounded_in?: { id: string; kind: string; name?: string; evidence?: Ev[] }[];
  provider?: string; model?: string; job?: Job | string; poll?: string; cached?: boolean; __status?: number;
};

export type WSBit = { joke_id: string; title: string; target_s: number; target_basis?: string };
export type WorkingSet = {
  id: string; title: string; storyboard_id: string; current: number; versions: { v: number; created_at: string; note: string; target_s: number; bits: WSBit[]; by: string }[];
  locks: Record<string, { bit: boolean; lines: { text: string; node: string; ts: string }[] }>; created_at?: string; updated_at?: string;
};
export type AlignBit = {
  joke_id: string; title: string; planned_pos: number; target_s: number; locked: boolean; performed: boolean; performed_pos?: number | null;
  time_s?: number | null; delta_s?: number | null; laugh_s?: number | null; laugh_s_per_min?: number | null; vs_room?: number | null; ai_rating?: string | null;
  time_to_first_laugh_s?: number | null; script_diff?: { on_script_pct: number; dropped: string[]; added: string[] } | null; in_order?: boolean;
};
export type Alignment = {
  working_set: string; version: number; set: string; date: string; venue: string; venue_label: string;
  order: { planned: string[]; performed: string[]; in_order: string[]; moved: string[]; same_order: boolean };
  dropped: string[]; added: { idx: number; title: string; status: string; joke_id: string | null; time_s: number; laugh_s_per_min: number; vs_room: number | null; ai_rating: string | null }[];
  bits: AlignBit[];
  totals: { planned_target_s: number; performed_planned_s: number; added_s: number; bits_performed: number; bits_planned: number; ws_laugh_s_per_min: number | null; set_laugh_s_per_min: number; room_heat: number; material_s: number };
};
export type TryNote = { id?: string; joke_id: string; title: string; kind: string; score?: number; text: string; evidence: Ev[]; source: Source; low_n?: boolean; done?: boolean };
export type Perfected = { locked_share: number; stability: number; perfected: number; formula: string; bits: { joke_id: string; title: string; locked_share: number; cv_laugh_s_per_min: number | null; stability: number; nights: number; low_n: boolean }[] };
export type WSStatus = { working_set: string; title: string; version: number; target_s: number; last_night: { set: string; date: string; venue: string; ws_laugh_s_per_min: number | null; bits_performed: number; bits_planned: number } | null; vs_previous: string | null; line: string; perfected?: number };
export type WSBitRow = WSBit & {
  locked: boolean; locked_lines: { text: string; node: string; ts: string }[]; nights: number; best_night: any; trend: string | null; stability: number | null; low_n: boolean;
  last_night: { performed: boolean; time_s: number | null; delta_s: number | null; laugh_s_per_min: number | null; vs_room: number | null; ai_rating: string | null } | null;
  notes: TryNote[];
};
export type WSView = { working_set: WorkingSet; version: WorkingSet['versions'][number]; status: WSStatus; perfected: Perfected; try_tonight: TryNote[]; bits: WSBitRow[]; __fixture?: boolean };
export type WSListItem = { id: string; title: string; version: number; target_s: number; n: number; storyboard_id: string | null; updated: string; source?: Source };
export type Job<T = any> = { id: string; kind: string; status: 'queued' | 'claimed' | 'done' | 'failed' | string; created_at?: string; done_at?: string | null; error?: string | null; result?: T | null; waiting_for?: 'transcriber' | string | null; note?: string | null };
export type AskAnswer = { question?: string; answer?: string; status?: string; sources?: { node: string; quote: string; title?: string; date?: string; set?: string; start_ms?: number | null; joke_id?: string }[]; jokes?: { id: string; title: string }[]; provider?: string; model?: string; not_found?: boolean };
/** A slow draft: 200 with the cached result, or 202 {status:"queued", job, poll}. */
export type Queued<T> = (T & { cached?: boolean; job?: Job }) | { status: 'queued'; job: Job; poll: string; waiting_for?: 'transcriber' | string | null; note?: string | null; __status?: number };
export type BitHistory = {
  joke_id: string; title: string; n_nights: number; nights: { date: string; set: string; venue: string; time_s: number; laugh_s: number; laugh_s_per_min: number; vs_room: number | null; ai_rating: string | null }[];
  best_night: any; trend: string | null; cv_laugh_s_per_min: number | null; stability: number; low_n: boolean;
  wording: { effects: { statement: string; delta_vs_room: number; n_with: number; n_without: number; low_n: boolean; examples: string[] }[]; changes: any[]; note: string };
  punch_up?: PunchUp | null; __fixture?: boolean;
};
export type PunchUp = {
  joke_id: string; title: string; draft: true; label: string; status: 'ok' | 'locked' | 'no_tape' | 'all_locked' | 'provider_error' | 'queued';
  drafts: { target_line: string; alt: string; move: string; draft: true; label: string }[]; provider?: string; model?: string; __status?: number;
};

export type Venue = {
  venue_id: string; raw: string; name: string; sets: { set: string; date: string; title: string; laugh_s_per_min: number; room_heat: number; pct_laughing: number; material_s: number; best: any; weakest: any }[];
  dates: string[]; n_sets: number; avg_laugh_s_per_min: number | null; avg_room_heat: number | null; best_jokes: any[]; worst_jokes: any[];
  works_here: { joke_id: string; title: string; delta?: number; vs_room_here?: number; n_here?: number; low_n?: boolean; [k: string]: any }[];
  works_elsewhere: any[]; note: string; geo?: { lat?: number; lon?: number; status?: string } | null;
};
export type VenuesView = { venues: Venue[]; ambiguous: { venue_id: string; raw: string; reasons: string[] }[]; geocoder?: { attribution?: string }; __fixture?: boolean };
export type GeoFeature = { type: 'Feature'; geometry: { type: 'Point'; coordinates: [number, number] } | null; properties: { venue_id: string; name: string; raw: string; status: string; ambiguous: boolean; n_sets: number; dates: string[]; avg_laugh_s_per_min: number | null; avg_room_heat: number | null; color: string; label: string } };
export type GeoJSON = { type: 'FeatureCollection'; features: GeoFeature[]; __fixture?: boolean };

export type TodayFeed = {
  date: string;
  latest_set: { set: string; date: string; venue: string; venue_label: string; title: string; material: string; laugh_s_per_min: number; room_heat: number; pct_laughing: number; jokes_counted: number; new_material: number; best: { joke_id: string; title: string } | null; weakest: any } | null;
  working_set: (WSStatus & { perfected?: number }) | null;
  try_tonight: TryNote[];
  serendipity: { kind: 'gem' | 'orphan'; joke_id: string; title: string; stage: string; why: string; score: number; source: Source; evidence: Ev[] }[];
  new_material: { idx: number; title: string; joke_id: string | null; ai_rating: string | null; laugh_s_per_min: number; vs_room: number | null; similar?: any; evidence: Ev[] }[];
  minutes: { working_min: number; testing_min: number; working_plus_testing_min: number; measured_share: number; n_working: number; n_testing: number; basis: string } | null;
  his_tonight_list: { joke_id: string; title: string; done: boolean }[];
  __fixture?: boolean;
};

export type Clip = {
  idx?: number; rank?: number; joke_id: string | null; title: string; start_ms: number; end_ms: number; duration_s?: number; status?: string; score?: number;
  score_breakdown?: Record<string, { raw: number; norm: number; weight: number; contribution: number }>; laugh_s?: number; vs_room?: number | null; why?: string;
};
export type ClipsView = { set: string; clips: Clip[]; source?: string | null; __derived?: boolean };

// ------------------------------------------------------------------ fallback plumbing
/** True when the route itself isn't there yet (vs a real error from a live route). */
export function routeMissing(e: unknown) {
  if (!(e instanceof ApiError)) return false;
  return e.status === 404 || e.status === 405 || e.status === 501;
}
async function withFixture<T>(live: () => Promise<T>, fixture: () => Promise<T>): Promise<T> {
  try {
    return await live();
  } catch (e) {
    if (DEV && routeMissing(e)) {
      const f: any = await fixture();
      if (f && typeof f === 'object') f.__fixture = true;
      return f as T;
    }
    throw e;
  }
}
const fx = () => (DEV ? import('./devFixtures') : Promise.reject(new Error('no fixtures in production')));
const enc = encodeURIComponent;
const asList = <T,>(j: any, key: string): T[] => (Array.isArray(j) ? j : Array.isArray(j?.[key]) ? j[key] : Array.isArray(j?.items) ? j.items : []);

// ------------------------------------------------------------------ calls
export const creative = {
  connections: (jokeId: string, cap = 10) =>
    withFixture(() => get<Connections>(`/api/jokes/${enc(jokeId)}/connections?cap=${cap}`), async () => (await fx()).connections(jokeId)),
  today: () => withFixture(() => get<TodayFeed>('/api/today').then(assertTodayFeed), async () => (await fx()).today()),
  boards: () => withFixture(() => get<any>('/api/boards').then((j) => ({ boards: asList<BoardListItem>(j, 'boards') })), async () => (await fx()).boards()),
  board: (id: string) => withFixture(() => get<BoardView>(`/api/boards/${enc(id)}`), async () => (await fx()).board(id)),
  createBoard: (b: { title: string; joke_ids: string[]; target_s?: number; notes?: string }) =>
    withFixture(() => post<any>('/api/boards', b).then(boardOf), async () => (await fx()).createBoard(b)),
  /** The put-away lists (`?archived=1`); no fixture: an empty list is the honest answer when the route is missing. */
  archivedBoards: () => get<any>('/api/boards?archived=1').then((j) => ({ boards: asList<BoardListItem>(j, 'boards') })),
  patchBoard: (id: string, p: BoardPatch) =>
    withFixture(() => patch<any>(`/api/boards/${enc(id)}`, p).then(boardOf), async () => (await fx()).patchBoard(id, p)),
  addItem: (id: string, jokeId: string, after?: string) =>
    withFixture(() => post<any>(`/api/boards/${enc(id)}/items`, { joke_id: jokeId, ...(after ? { after } : {}) }).then(boardOf), async () => (await fx()).addItem(id, jokeId, after)),
  bridge: (id: string, from: string, to: string) =>
    withFixture(() => post<any>(`/api/boards/${enc(id)}/bridge`, { from, to }).then((b) => (b.__status === 202 || b.status === 'queued' ? ({ ...b, from, to, draft: true, label: 'DRAFT · AI · not your words yet', status: 'queued' } as Bridge) : (b as Bridge))), async () => (await fx()).bridge(from, to)),
  workingsets: () => withFixture(() => get<any>('/api/workingsets').then((j) => ({ working_sets: asList<WSListItem>(j?.working_sets ? j : { working_sets: j?.workingsets }, 'working_sets') })), async () => (await fx()).workingsets()),
  workingset: (id: string) => withFixture(() => get<WSView>(`/api/workingsets/${enc(id)}`), async () => (await fx()).workingset(id)),
  promote: (boardId: string, targetS: number) =>
    withFixture(() => post<WSView>('/api/workingsets', { board_id: boardId, target_s: targetS }), async () => (await fx()).promote(boardId, targetS)),
  newVersion: (id: string, note: string, jokeIds: string[]) =>
    withFixture(() => post<WSView>(`/api/workingsets/${enc(id)}/versions`, { note, joke_ids: jokeIds }), async () => (await fx()).newVersion(id, note, jokeIds)),
  lock: (id: string, b: { joke_id: string; line?: string; unlock?: boolean }) =>
    withFixture(() => post<WSView>(`/api/workingsets/${enc(id)}/lock`, b), async () => (await fx()).lock(id, b)),
  punchup: (wsId: string, jokeId: string, fresh = false) =>
    withFixture(() => post<Queued<PunchUp>>(`/api/workingsets/${enc(wsId)}/bits/${enc(jokeId)}/punchup`, fresh ? { fresh: true } : {}), async () => (await fx()).punchup(wsId, jokeId)),
  ask: (question: string) => withFixture(() => post<Queued<AskAnswer>>('/api/ask', { question }), async () => (await fx()).ask(question)),
  job: <T = any>(id: string) => withFixture(() => get<Job<T>>(`/api/jobs/${enc(id)}`), async () => (await fx()).job(id) as any),
  nights: (id: string) => withFixture(() => get<any>(`/api/workingsets/${enc(id)}/nights`).then((j) => ({ nights: asList<Alignment>(j, 'nights') })), async () => (await fx()).nights(id)),
  bit: (id: string, jokeId: string) => withFixture(() => get<BitHistory>(`/api/workingsets/${enc(id)}/bits/${enc(jokeId)}`), async () => (await fx()).bit(id, jokeId)),
  venues: () => withFixture(() => get<VenuesView>('/api/venues'), async () => (await fx()).venues()),
  geojson: () => withFixture(() => get<GeoJSON>('/api/venues.geojson'), async () => (await fx()).geojson()),
  patchVenue: (id: string, p: { name?: string; lat?: number; lon?: number }) =>
    withFixture(() => patch<any>(`/api/venues/${enc(id)}`, p), async () => (await fx()).patchVenue(id, p)),
  clips: (setId: string) => get<ClipsView>(`/api/sets/${enc(setId)}/clips`),
};

function boardOf(j: any): BoardView {
  // a write may answer with the board view or just the board; callers re-fetch the view when they need it
  return j && j.board ? j : { board: j, health: null as any, best_next: [], setlist: null as any };
}
function assertTodayFeed(j: any): TodayFeed {
  // until the server ships today.build, /api/today is the older feed (latest_analysis, glance): treat as missing
  if (!j || !('minutes' in j || 'serendipity' in j || 'try_tonight' in j)) throw new ApiError(404, 'today feed not live');
  return j as TodayFeed;
}

/** useApi over a creative call, keyed by `key`. `fixture` = this data came from the dev fixture fallback. */
export function useCreative<T>(key: string | null, call: () => Promise<T>, opts?: { poll?: number }) {
  // `call` closes over ids that are part of `key`; keep the fetcher stable per key
  const fetcher = useCallback(() => call(), [key]); // eslint-disable-line react-hooks/exhaustive-deps
  const r = useApi<T>(key ? `creative:${key}` : null, { fetcher, poll: opts?.poll, cache: true });
  return { ...r, fixture: !!(r.data as any)?.__fixture };
}
