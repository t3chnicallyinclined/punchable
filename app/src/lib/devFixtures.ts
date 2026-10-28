// DEV ONLY: stands in for docs/API_CONTRACT.md routes that the server hasn't shipped yet.
// Reads the creative layer's own demo outputs from /dev-fixtures/ (npm run dev-fixtures copies them; gitignored)
// and keeps writes in memory for this tab, following creative/story.py and workingset.py semantics closely enough
// to drive the screens. Nothing here ever runs in a production build (src/lib/creative.ts gates it on __DEV__).
import Constants from 'expo-constants';
import { Platform } from 'react-native';
import { get } from './api';
import type {
  Alignment, BestNext, BitHistory, BoardBit, BoardListItem, BoardView, Bridge, Connections, GeoJSON, Health, Transition, TodayFeed,
  VenuesView, WSView, WorkingSet,
} from './creative';

function base() {
  if (Platform.OS === 'web') return '';
  const host = (Constants.expoConfig as any)?.hostUri as string | undefined;
  return host ? `http://${host}` : '';
}
const files = new Map<string, Promise<any>>();
function file<T = any>(name: string): Promise<T> {
  if (!files.has(name)) {
    files.set(name, fetch(`${base()}/dev-fixtures/${name}`).then((r) => {
      if (!r.ok) throw new Error(`dev fixture ${name} missing: run \`npm run dev-fixtures\``);
      return r.json();
    }));
  }
  return files.get(name)!;
}
const clone = <T,>(x: T): T => JSON.parse(JSON.stringify(x));
const slug = (id: string) => id.replace(/^joke:/, '');

// ------------------------------------------------------------------ indexes built once from the fixtures
type Index = { bits: Map<string, BoardBit>; pairs: Map<string, Transition>; views: Map<string, BoardView> };
let index: Promise<Index> | null = null;
function idx(): Promise<Index> {
  if (!index) {
    index = (async () => {
      const seeded: BoardView[] = await file('storyboards_seeded.json');
      const snap = await file<any>('storyboards.json').catch(() => ({ items: {} }));
      const ws = await file<any>('workingset_demo.json');
      const bits = new Map<string, BoardBit>();
      const pairs = new Map<string, Transition>();
      const views = new Map<string, BoardView>();
      for (const v of seeded) {
        for (const b of v.health.bits) bits.set(b.joke_id, b);
        for (const t of v.health.transitions) pairs.set(`${t.from}>${t.to}`, t);
        views.set(v.board.id, clone(v));
      }
      // performed bits of the demo working set carry their measured energy
      for (const h of ws.history || []) {
        if (!bits.has(h.joke_id)) {
          const e = h.nights?.length ? h.nights.reduce((a: number, n: any) => a + Math.min(3, n.vs_room || 0), 0) / h.nights.length : null;
          const dur = h.nights?.length ? h.nights.map((n: any) => n.time_s).sort((a: number, b: number) => a - b)[Math.floor(h.nights.length / 2)] : 60;
          bits.set(h.joke_id, { joke_id: h.joke_id, title: h.title, stage: 'working', duration_s: dur, duration_basis: `tape (${h.n_nights} nights)`, energy: e, heat: heat(e), nights: h.n_nights, devices: [] });
        }
      }
      // boards in the store snapshot that weren't seeded (the demo run, his own)
      for (const b of Object.values<any>(snap.items || {})) {
        if (!views.has(b.id) && !b.deleted) views.set(b.id, await viewOf(clone(b), { bits, pairs, views }));
      }
      return { bits, pairs, views };
    })();
  }
  return index;
}
const heat = (e: number | null | undefined) => (e == null ? 0 : e < 0.5 ? 1 : e < 0.9 ? 2 : e < 1.3 ? 3 : 4);
const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, '0')}`;

async function bitOf(i: Pick<Index, 'bits'>, id: string): Promise<BoardBit> {
  const b = i.bits.get(id);
  if (b) return b;
  let title = slug(id);
  let stage = 'idea';
  try {
    const d = await get<any>(`/api/jokes/${encodeURIComponent(id)}`);
    title = d.joke?.title || title;
    stage = d.joke?.stage || stage;
  } catch {}
  const nb: BoardBit = { joke_id: id, title, stage, duration_s: 60, duration_basis: 'estimate (no tape)', energy: null, heat: 0, nights: 0, devices: [] };
  i.bits.set(id, nb);
  return nb;
}
function transition(i: Pick<Index, 'pairs'>, a: string, b: string): Transition {
  return i.pairs.get(`${a}>${b}`) || { from: a, to: b, label: 'jump', why: 'no link between these two yet', via: [], needs_bridge: true, shared_topics: [], nights: [], evidence: [] };
}
async function viewOf(board: any, i: Pick<Index, 'bits' | 'pairs' | 'views'>): Promise<BoardView> {
  const bits = await Promise.all(board.joke_ids.map((j: string) => bitOf(i, j)));
  const transitions = bits.slice(1).map((b, k) => transition(i, bits[k].joke_id, b.joke_id));
  const runtime_s = bits.reduce((a, b) => a + (b.duration_s || 0), 0);
  const target = board.target_s || 600;
  const checks: Health['checks'] = [];
  const off = (runtime_s - target) / target;
  checks.push({ rule: 'runtime', status: Math.abs(off) <= 0.1 ? 'ok' : off > 0 ? 'warn' : 'info', text: Math.abs(off) <= 0.1 ? `${fmt(runtime_s)} of ${fmt(target)}: on target` : off > 0 ? `${fmt(runtime_s)}: ${fmt(runtime_s - target)} over ${fmt(target)}` : `${fmt(runtime_s)}: ${fmt(target - runtime_s)} under ${fmt(target)}` });
  const jumps = transitions.filter((t) => t.label === 'jump');
  checks.push({ rule: 'jumps', status: jumps.length ? 'warn' : 'ok', text: jumps.length ? `${jumps.length} ${jumps.length === 1 ? 'jump' : 'jumps'} with no link` : 'every transition has a link' });
  const rep = bits.slice(1).filter((b, k) => b.devices.slice(0, 3).some((d: string) => bits[k].devices.slice(0, 3).includes(d)));
  checks.push({ rule: 'repeated_devices', status: rep.length ? 'warn' : 'ok', text: rep.length ? `the same move twice in a row before ${rep[0].title}` : 'no move repeats back-to-back' });
  const last = bits[bits.length - 1];
  const strong = last && last.energy != null && last.energy >= 1.2 && last.nights >= 2;
  const best = [...bits].filter((b) => b.energy != null).sort((a, b) => (b.energy || 0) - (a.energy || 0))[0];
  checks.push({ rule: 'closer', status: !last || strong ? 'ok' : 'warn', text: !last ? 'no bits yet' : strong ? `ends on ${last.title}, ${last.energy!.toFixed(1)}× the room` : `ends soft: ${last.title}${best && best !== last ? ` · ${best.title} is stronger` : ''}`, suggest: best?.joke_id });
  const dip = bits.slice(1).some((b, k) => (b.energy ?? 9) < 0.8 && (bits[k].energy ?? 9) < 0.8);
  checks.push({ rule: 'energy_dip', status: dip ? 'warn' : 'ok', text: dip ? 'two quiet bits in a row' : 'no energy dip' });
  const health: Health = {
    board: board.id, title: board.title, runtime_s, runtime: fmt(runtime_s), target_s: target, bits, transitions,
    energy_curve: bits.map((b) => ({ joke_id: b.joke_id, energy: b.energy, heat: b.heat })), checks, untaped: bits.filter((b) => !b.nights).map((b) => b.joke_id),
  };
  const prev = i.views.get(board.id);
  let best_next: BestNext[] = prev?.best_next?.filter((x) => !board.joke_ids.includes(x.joke_id)) || [];
  if (best_next.length < 3) {
    const pool = [...i.bits.values()].filter((b) => !board.joke_ids.includes(b.joke_id) && b.energy != null && b.stage !== 'retired')
      .sort((a, b) => (b.energy || 0) - (a.energy || 0)).slice(0, 3 - best_next.length);
    best_next = best_next.concat(pool.map((b) => ({
      joke_id: b.joke_id, title: b.title, stage: b.stage, score: 0.25, why: `strong on tape (${(b.energy || 0).toFixed(2)}x the room over ${b.nights} nights)`,
      source: 'inferred' as const, transition: last ? transition(i, last.joke_id, b.joke_id).label : 'jump', energy: b.energy, nights: b.nights, evidence: [],
    })));
  }
  const lines = bits.map((b, k) => `${k + 1}. ${b.title} (${fmt(b.duration_s)})`);
  return { board, health, best_next, setlist: { board: board.id, title: board.title, lines, runtime: fmt(runtime_s), text: `${board.title} · ${fmt(runtime_s)}\n${lines.join('\n')}` } };
}

// ------------------------------------------------------------------ routes
export async function connections(jokeId: string): Promise<Connections> {
  try {
    return clone(await file<Connections>(`connections_${slug(jokeId)}.json`));
  } catch {
    let title = slug(jokeId);
    try {
      title = (await get<any>(`/api/jokes/${encodeURIComponent(jokeId)}`)).joke?.title || title;
    } catch {}
    return {
      joke: { id: jokeId, title, stage: 'idea', stage_source: 'inferred', summary: '', devices: [], duration_s: 0, duration_basis: '', rooms: 0, nights: 0 },
      groups: [], stage_trend: { nights: [], trend: null, avg_laugh_s_per_min: null, avg_vs_room: null, best_night: null, low_n: true }, land_harder: [], notes: ['no fixture for this joke'],
    };
  }
}
export async function today(): Promise<TodayFeed> {
  return clone(await file<TodayFeed>('today.json'));
}
export async function boards(): Promise<{ boards: BoardListItem[] }> {
  const i = await idx();
  const wss = await wsState();
  const out = [...i.views.values()].filter((v) => !v.board.deleted).map((v) => ({
    id: v.board.id, title: v.board.title, n: v.board.joke_ids.length, runtime: v.health.runtime, updated: v.board.updated_at, target_s: v.board.target_s, source: v.board.source,
    working_set: [...wss.values()].find((w) => w.storyboard_id === v.board.id)?.id || null,
  }));
  out.sort((a, b) => (b.updated || '').localeCompare(a.updated || ''));
  return { boards: out };
}
export async function board(id: string): Promise<BoardView> {
  const i = await idx();
  const v = i.views.get(id);
  if (!v) throw new Error('no such board');
  return clone(v);
}
const now = () => new Date().toISOString();
export async function createBoard(b: { title: string; joke_ids: string[]; target_s?: number; notes?: string }): Promise<BoardView> {
  const i = await idx();
  const id = `sb:${b.title.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 30)}-${Date.now().toString(16).slice(-6)}`;
  const board = { id, title: b.title, joke_ids: b.joke_ids, target_s: b.target_s || 600, notes: b.notes || '', source: 'his', by: 'owner', created_at: now(), updated_at: now(), deleted: false };
  const v = await viewOf(board, i);
  i.views.set(id, v);
  return clone(v);
}
export async function patchBoard(id: string, p: { title?: string; joke_ids?: string[]; target_s?: number; notes?: string; add?: { joke_id: string; at?: number }; remove?: { joke_id: string } }): Promise<BoardView> {
  const i = await idx();
  const v = i.views.get(id);
  if (!v) throw new Error('no such board');
  const b = { ...v.board, updated_at: now(), source: 'his' as const, by: 'owner' };
  if (p.title != null) b.title = p.title;
  if (p.target_s != null) b.target_s = p.target_s;
  if (p.notes != null) b.notes = p.notes;
  if (p.joke_ids) b.joke_ids = p.joke_ids.slice();
  if (p.add && !b.joke_ids.includes(p.add.joke_id)) b.joke_ids.splice(p.add.at ?? b.joke_ids.length, 0, p.add.joke_id);
  if (p.remove) b.joke_ids = b.joke_ids.filter((x) => x !== p.remove!.joke_id);
  const nv = await viewOf(b, i);
  i.views.set(id, nv);
  return clone(nv);
}
export async function addItem(id: string, jokeId: string, after?: string): Promise<BoardView> {
  const v = (await idx()).views.get(id);
  const at = after && v ? v.board.joke_ids.indexOf(after) + 1 : undefined;
  return patchBoard(id, { add: { joke_id: jokeId, at } });
}
const bridgeAsked = new Map<string, number>();
export async function bridge(from: string, to: string): Promise<Bridge> {
  const f = await file<Bridge>('bridge_draft.json');
  if (f.from === from && f.to === to) return clone(f);
  // other pairs: the writer hasn't drafted it; the real route answers 202 and the draft arrives later
  const k = `${from}>${to}`;
  if (!bridgeAsked.has(k)) bridgeAsked.set(k, Date.now());
  return { from, to, draft: true, label: 'DRAFT · AI · not your words yet', status: 'queued', job: `job:dev-${k.length}`, __status: 202 };
}

// working sets
let wsStore: Promise<Map<string, WorkingSet>> | null = null;
function wsState() {
  if (!wsStore) wsStore = file<any>('workingset_demo.json').then((d) => new Map([[d.working_set.id, clone(d.working_set) as WorkingSet]]));
  return wsStore;
}
export async function workingsets() {
  const out = [...(await wsState()).values()].map((w) => {
    const cur = w.versions.find((v) => v.v === w.current)!;
    return { id: w.id, title: w.title, version: w.current, target_s: cur.target_s, n: cur.bits.length, storyboard_id: w.storyboard_id, updated: w.updated_at || '', source: 'his' as const };
  });
  return { working_sets: out };
}
export async function workingset(id: string): Promise<WSView> {
  const ws = (await wsState()).get(id);
  if (!ws) throw new Error('no such working set');
  const d = await file<any>('workingset_demo.json');
  const demo = d.working_set.id === id;
  const cur = ws.versions.find((v) => v.v === ws.current)!;
  const lockedSecs = cur.bits.reduce((a, b) => a + (ws.locks[b.joke_id]?.bit ? b.target_s : 0), 0);
  const total = cur.bits.reduce((a, b) => a + b.target_s, 0) || 1;
  const perfected = demo ? clone(d.perfected) : { locked_share: 0, stability: 0, perfected: 0, formula: '', bits: [] };
  perfected.locked_share = lockedSecs / total;
  const status = demo ? clone(d.status) : { working_set: id, title: ws.title, version: ws.current, target_s: cur.target_s, last_night: null, vs_previous: null, line: `${ws.title} v${ws.current}` };
  status.version = ws.current;
  const tries = (demo ? clone(d.try_tonight) : []).filter((t: any) => !ws.locks[t.joke_id]?.bit);
  const aligned = demo ? (d.alignments as Alignment[]).filter((a) => a.totals.bits_performed) : [];
  const lastBits = new Map((aligned[aligned.length - 1]?.bits || []).map((b) => [b.joke_id, b]));
  const hist = new Map<string, any>((demo ? d.history : []).map((h: any) => [h.joke_id, h]));
  const bits = cur.bits.map((b) => {
    const h = hist.get(b.joke_id) || {};
    const lb: any = lastBits.get(b.joke_id);
    const lk = ws.locks[b.joke_id] || { bit: false, lines: [] };
    return { ...b, locked: !!lk.bit, locked_lines: lk.lines, nights: h.n_nights || 0, best_night: h.best_night || null, trend: h.trend || null, stability: h.stability ?? null, low_n: h.low_n ?? true,
      last_night: lb ? { performed: lb.performed, time_s: lb.time_s ?? null, delta_s: lb.delta_s ?? null, laugh_s_per_min: lb.laugh_s_per_min ?? null, vs_room: lb.vs_room ?? null, ai_rating: lb.ai_rating ?? null } : null,
      notes: tries.filter((t: any) => t.joke_id === b.joke_id) };
  });
  return { working_set: clone(ws), version: clone(cur), status, perfected, try_tonight: tries, bits };
}
export async function promote(boardId: string, targetS: number): Promise<WSView> {
  const v = await board(boardId);
  const id = `ws:${boardId.replace(/^sb:/, '')}`;
  const ws: WorkingSet = {
    id, title: v.board.title, storyboard_id: boardId, current: 1, locks: {}, created_at: now(), updated_at: now(),
    versions: [{ v: 1, created_at: now(), note: `v1: promoted from storyboard '${v.board.title}'`, target_s: targetS, by: 'owner',
      bits: v.health.bits.map((b) => ({ joke_id: b.joke_id, title: b.title, target_s: Math.max(5, Math.round(b.duration_s / 5) * 5), target_basis: b.duration_basis })) }],
  };
  (await wsState()).set(id, ws);
  return workingset(id);
}
export async function newVersion(id: string, note: string, jokeIds: string[]): Promise<WSView> {
  if (!note.trim()) throw new Error('a new version needs a note saying what changed');
  const ws = (await wsState()).get(id)!;
  const cur = ws.versions.find((v) => v.v === ws.current)!;
  const byId = new Map(cur.bits.map((b) => [b.joke_id, b]));
  const i = await idx();
  const bits = await Promise.all(jokeIds.map(async (j) => byId.get(j) || { joke_id: j, title: (await bitOf(i, j)).title, target_s: 60 }));
  ws.versions.push({ v: ws.current + 1, created_at: now(), note, target_s: cur.target_s, bits, by: 'owner' });
  ws.current += 1;
  ws.updated_at = now();
  return workingset(id);
}
export async function lock(id: string, b: { joke_id: string; line?: string; unlock?: boolean }): Promise<WSView> {
  const ws = (await wsState()).get(id)!;
  const l = (ws.locks[b.joke_id] ||= { bit: false, lines: [] });
  if (b.line) {
    if (b.unlock) l.lines = l.lines.filter((x) => x.text !== b.line);
    else if (!l.lines.some((x) => x.text === b.line)) l.lines.push({ text: b.line, node: '', ts: now() });
  } else l.bit = !b.unlock;
  return workingset(id);
}
export async function nights(id: string): Promise<{ nights: Alignment[] }> {
  const d = await file<any>('workingset_demo.json');
  return { nights: d.working_set.id === id ? clone(d.alignments) : [] };
}
export async function bit(id: string, jokeId: string): Promise<BitHistory> {
  const d = await file<any>('workingset_demo.json');
  const h = (d.history || []).find((x: any) => x.joke_id === jokeId);
  const ws = (await wsState()).get(id);
  const lk = ws?.locks[jokeId] || { bit: false, lines: [] };
  const extra = { working_set: id, locked: !!lk.bit, locked_lines: lk.lines };
  if (!h) return { joke_id: jokeId, title: slug(jokeId), n_nights: 0, nights: [], best_night: null, trend: null, cv_laugh_s_per_min: null, stability: 0, low_n: true, wording: { effects: [], changes: [], note: '' }, ...extra } as any;
  return { ...clone(h), ...extra };
}
export async function punchup(id: string, jokeId: string): Promise<any> {
  const d = await file<any>('workingset_demo.json');
  const ws = (await wsState()).get(id);
  if (ws?.locks[jokeId]?.bit) return { joke_id: jokeId, title: slug(jokeId), draft: true, label: 'DRAFT · AI · not your words yet', status: 'locked', drafts: [] };
  if (d.punch_up?.joke_id === jokeId) return { cached: true, ...clone(d.punch_up) };
  return { status: 'queued', job: { id: `job:dev-punchup-${slug(jokeId)}`, kind: 'punchup', status: 'queued' }, poll: '', __status: 202 };
}
export async function ask(question: string): Promise<any> {
  return { status: 'queued', job: { id: `job:dev-ask-${question.length}`, kind: 'ask', status: 'queued' }, poll: '', __status: 202 };
}
export async function job(id: string): Promise<any> {
  return { id, kind: id.includes('ask') ? 'ask' : 'draft', status: 'queued', result: null };
}
let venueEdits: Record<string, { name?: string; lat?: number; lon?: number }> = {};
export async function venues(): Promise<VenuesView> {
  const v = clone(await file<VenuesView>('venues.json'));
  for (const x of v.venues) if (venueEdits[x.venue_id]?.name) x.name = venueEdits[x.venue_id].name!;
  return v;
}
export async function geojson(): Promise<GeoJSON> {
  const g = clone(await file<GeoJSON>('venues.geojson.json'));
  for (const f of g.features) {
    const e = venueEdits[f.properties.venue_id];
    if (e?.name) f.properties.name = e.name;
    if (e?.lat != null && e?.lon != null) f.geometry = { type: 'Point', coordinates: [e.lon, e.lat] };
  }
  return g;
}
export async function patchVenue(id: string, p: { name?: string; lat?: number; lon?: number }) {
  venueEdits = { ...venueEdits, [id]: { ...(venueEdits[id] || {}), ...p } };
  return { ok: true };
}
