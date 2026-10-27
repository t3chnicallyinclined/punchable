// Client for the Punchable API (api-dev/API.md + docs/API_CONTRACT.md). The hosted API keeps the same shapes.
import { useCallback, useEffect, useRef, useState } from 'react';
import { useFocusEffect } from 'expo-router';

// Web only: ?api=http://127.0.0.1:<port> points this tab at another local API (the tests use a temp-kb API this way).
function runtimeApi(): string | null {
  try {
    const loc = (globalThis as any).location;
    const ss = (globalThis as any).sessionStorage;
    if (!loc || !ss) return null;
    const q = new URLSearchParams(loc.search).get('api');
    if (q && /^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(q)) ss.setItem('punchable.api', q);
    return ss.getItem('punchable.api');
  } catch {
    return null;
  }
}
// EXPO_PUBLIC_API_URL may be the hosted base with "/api" already on it (infra/web/Dockerfile: https://punchable.io/api).
// Paths in this app always start with "/api/", so a trailing "/api" on the base is dropped.
export const API_URL = (runtimeApi() || process.env.EXPO_PUBLIC_API_URL || 'http://127.0.0.1:8796').replace(/\/$/, '').replace(/\/api$/, '');

// ---- auth: the login module (src/lib/auth*, owned by the server agent) registers a header provider here.
type HeaderProvider = () => Record<string, string> | Promise<Record<string, string>>;
let authHeaders: HeaderProvider | null = null;
/** Called by src/lib/auth* once a bearer token exists, e.g. `setAuthHeaderProvider(() => ({ Authorization: 'Bearer …' }))`. */
export function setAuthHeaderProvider(p: HeaderProvider | null) {
  authHeaders = p;
}
async function headers(extra?: Record<string, string>) {
  const h = authHeaders ? await authHeaders() : {};
  return { ...h, ...(extra || {}) };
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, msg: string) {
    super(msg);
    this.status = status;
  }
}

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const r = await fetch(API_URL + path, {
    method,
    headers: await headers(body !== undefined ? { 'Content-Type': 'application/json' } : undefined),
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new ApiError(r.status, (j && j.error) || `HTTP ${r.status}`);
  if (r.status === 202 && j && typeof j === 'object') (j as any).__status = 202;
  return j as T;
}
export const get = <T = any>(path: string) => req<T>('GET', path);
export const post = <T = any>(path: string, body: unknown) => req<T>('POST', path, body);
export const patch = <T = any>(path: string, body: unknown) => req<T>('PATCH', path, body);
export const del = <T = any>(path: string) => req<T>('DELETE', path);

export type Written = { ok: boolean; indexes: number[]; decisions: any[] };

export const decide = (d: { target: 'edge' | 'node'; action: string; id?: string; after?: any; reason?: string }) =>
  post<Written>('/api/decision', d);
export const undo = (indexes: number[]) => post<Written>('/api/undo', { indexes });

export type UploadMeta = {
  kind: 'set' | 'note' | 'upload'; filename: string; title?: string; durationS?: number; marksMs?: number[];
  /** Requested by app (docs/API_CONTRACT.md): stored in the inbox meta.json when the API supports them. */
  recordedAt?: string; venue?: string; lat?: number; lng?: number; showGroup?: string; alignWorkingSet?: string;
};
export async function upload(blob: Blob, opts: UploadMeta) {
  const enc = encodeURIComponent;
  const r = await fetch(API_URL + '/api/upload', {
    method: 'POST',
    headers: await headers({
      'Content-Type': blob.type || 'application/octet-stream',
      'X-Kind': opts.kind,
      'X-Filename': enc(opts.filename),
      ...(opts.title ? { 'X-Title': enc(opts.title) } : {}),
      ...(opts.durationS ? { 'X-Duration-S': String(Math.round(opts.durationS)) } : {}),
      ...(opts.marksMs?.length ? { 'X-Marks-Ms': JSON.stringify(opts.marksMs) } : {}),
      ...(opts.recordedAt ? { 'X-Recorded-At': opts.recordedAt } : {}),
      ...(opts.venue ? { 'X-Venue': enc(opts.venue) } : {}),
      ...(opts.lat != null && opts.lng != null ? { 'X-Lat': String(opts.lat), 'X-Lng': String(opts.lng) } : {}),
      ...(opts.showGroup ? { 'X-Show-Group': enc(opts.showGroup) } : {}),
      ...(opts.alignWorkingSet ? { 'X-Align-Working-Set': enc(opts.alignWorkingSet) } : {}),
    }),
    body: blob,
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new ApiError(r.status, j.error || `HTTP ${r.status}`);
  return j as { status: 'queued'; file: string; kind: string; bytes: number };
}

// ---- phone cache: the last good response per path, so screens render instantly (and offline)
const CACHE_PREFIX = 'pk.cache.v1:';
const mem = new Map<string, any>();
export function cacheRead<T>(path: string): T | null {
  const k = API_URL + path;
  if (mem.has(k)) return mem.get(k);
  try {
    const raw = (globalThis as any).localStorage?.getItem(CACHE_PREFIX + k);
    if (raw) {
      const v = JSON.parse(raw);
      mem.set(k, v);
      return v;
    }
  } catch {}
  return null;
}
export function cacheWrite(path: string, v: unknown) {
  const k = API_URL + path;
  mem.set(k, v);
  try {
    const s = JSON.stringify(v);
    if (s.length < 400_000) (globalThis as any).localStorage?.setItem(CACHE_PREFIX + k, s);
  } catch {}
}

// ---- offline state, shared
let online = true;
const onlineListeners = new Set<(b: boolean) => void>();
export function setOnline(b: boolean) {
  if (b === online) return;
  online = b;
  onlineListeners.forEach((f) => f(b));
}
export function useOnline() {
  const [o, setO] = useState(online);
  useEffect(() => {
    onlineListeners.add(setO);
    return () => {
      onlineListeners.delete(setO);
    };
  }, []);
  return o;
}

// ---- live updates: one shared WebSocket to /api/live (SurrealDB LIVE SELECT on the API side)
type LiveMsg = { type: string; rev?: string | null };
const liveListeners = new Set<(m: LiveMsg) => void>();
let liveSock: WebSocket | null = null;
let liveUp = false;
let liveOk: boolean | null = null; // /api/live needs the SurrealDB backend; ask once instead of failing handshakes
function liveConnect() {
  if (liveSock || typeof WebSocket === 'undefined' || !liveListeners.size || liveOk === false) return;
  if (liveOk === null) {
    liveOk = false;
    get<{ primary?: string }>('/api/backend').then((b) => { liveOk = b?.primary === 'surreal'; if (liveOk) liveConnect(); }).catch(() => { liveOk = null; });
    return;
  }
  let ws: WebSocket;
  try {
    ws = new WebSocket(API_URL.replace(/^http/, 'ws') + '/api/live');
  } catch {
    return;
  }
  liveSock = ws;
  ws.onmessage = (e) => {
    try {
      const m = JSON.parse(String(e.data)) as LiveMsg;
      liveUp = true;
      if (m.type === 'change') liveListeners.forEach((f) => f(m));
    } catch {
      /* not JSON: ignore */
    }
  };
  ws.onclose = () => {
    liveSock = null;
    liveUp = false;
    setTimeout(liveConnect, 5000); // the API restarted or uses the files backend: retry; polling covers the gap
  };
  ws.onerror = () => ws.close();
}
function subscribeLive(f: (m: LiveMsg) => void) {
  liveListeners.add(f);
  liveConnect();
  return () => {
    liveListeners.delete(f);
  };
}

/**
 * Fetch on mount (rendering the cached copy first), again whenever the screen regains focus, and on reload().
 * With poll (ms), also re-fetch when the data changes: the API pushes over /api/live; only while that socket is
 * down does it fall back to polling /api/health's rev every `poll` ms (visible pages only).
 * `fetcher` replaces the plain GET (the creative client uses it for its dev fixture fallback).
 */
export function useApi<T = any>(path: string | null, opts?: { poll?: number; fetcher?: (path: string) => Promise<T>; cache?: boolean }) {
  const useCache = opts?.cache !== false;
  const [data, setData] = useState<T | null>(() => (path && useCache ? cacheRead<T>(path) : null));
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [stale, setStale] = useState(false);
  const seq = useRef(0);
  const fetcher = opts?.fetcher;
  const load = useCallback(async () => {
    if (!path) return;
    const n = ++seq.current;
    setLoading(true);
    try {
      const d = await (fetcher ? fetcher(path) : get<T>(path));
      if (n === seq.current) {
        setData(d);
        setError(null);
        setStale(false);
        setOnline(true);
        if (useCache && !(d as any)?.__fixture) cacheWrite(path, d);
      }
    } catch (e: any) {
      if (n === seq.current) {
        setError(e?.message || String(e));
        if (!(e instanceof ApiError)) setOnline(false);
        const c = useCache ? cacheRead<T>(path) : null;
        if (c) {
          setData(c);
          setStale(true);
        }
      }
    } finally {
      if (n === seq.current) setLoading(false);
    }
  }, [path, fetcher, useCache]);
  useEffect(() => {
    if (path && useCache) {
      const c = cacheRead<T>(path);
      if (c) setData(c);
    }
    load();
  }, [load]); // eslint-disable-line react-hooks/exhaustive-deps
  const rev = useRef<string | null>(null);
  useEffect(() => {
    if (!opts?.poll || !path) return;
    let alive = true;
    const unsub = subscribeLive(() => {
      if (alive) load();
    });
    const tick = async () => {
      const doc = (globalThis as any).document;
      if (liveUp || (doc && doc.visibilityState === 'hidden')) return;
      try {
        const h = await get<{ rev?: string }>('/api/health');
        setOnline(true);
        if (!alive || !h.rev) return;
        if (rev.current && rev.current !== h.rev) load();
        rev.current = h.rev;
      } catch {
        setOnline(false);
      }
    };
    tick();
    const t = setInterval(tick, opts.poll);
    return () => {
      alive = false;
      unsub();
      clearInterval(t);
    };
  }, [opts?.poll, path, load]);
  const first = useRef(true);
  useFocusEffect(
    useCallback(() => {
      if (first.current) {
        first.current = false;
        return;
      }
      load();
    }, [load]),
  );
  return { data, error: data && stale ? null : error, rawError: error, loading, reload: load, setData, stale };
}

// ---- shapes (subset the UI uses) ----
export type Evidence = { node: string; quote: string; start_ms?: number | null; field?: string };
export type Brief = { id: string; type: string; title: string; kind?: string; stage?: string; definition?: string; date?: string; doc_title?: string };
/** ai = inferred by the AI and untouched (shown as fact); suggested = a proposal; the rest are the comic's. */
export type EdgeState = 'ai' | 'suggested' | 'confirmed' | 'edited' | 'yours' | 'rejected' | 'dismissed';
export type EdgeView = {
  id: string; src: string; dst: string; kind: string; display_kind: string; status: string; bucket: 'proposed' | 'confirmed' | 'rejected';
  state: EdgeState; ai?: boolean;
  confidence?: number; reason?: string; evidence?: Evidence[]; yours?: boolean; edited?: boolean; other?: Brief; via_unit?: Brief; direction?: string;
};
export type Laugh = { id: string; start_ms: number; end_ms: number; strength: number; bars: number; kind: string; confidence: number };
export type JokeSummary = {
  id: string; title: string; stage: string; summary: string; versions: number; sets: number; hit_sets: number; to_review: number;
  first_seen?: string; last_seen?: string; labels: { id: string; name: string; bucket: string }[];
  ai_hit_sets?: number; stage_ai?: boolean; stage_confidence?: number | null; who?: 'ai' | 'confirmed' | 'edited';
  // Requested by app (docs/API_CONTRACT.md); optional until the API returns them.
  short_title?: string; punch?: string; nights?: number; avg_vs_room?: number | null; avg_laugh_s_per_min?: number | null; last_performed?: string | null;
};
export const setRow = (b: { perf: string; op: 'rate' | 'match' | 'new' | 'crowd' | 'bit' | 'confirm'; rating?: string; joke?: string; reason?: string }) =>
  post<Written>('/api/set-row', b);
export type Step = {
  key: string; kind: 'joke' | 'new_line' | 'crowd_work'; perf: any; name?: string | null;
  joke: { id: string; title: string; stage: string; sets: number; hit_sets: number } | null; match: EdgeView | null;
  text: string; punch: { text: string; start_ms: number; end_ms: number; laugh: Laugh | null; segments: { start_ms: number; end_ms: number; text: string; punch: boolean }[] };
  laughs: Laugh[]; max_bars: number; hits: number; suggested: string; rating: string | null; ai_rating?: string | null; play: { start_ms: number; end_ms: number };
};

export type SetCard = {
  id: string; title: string; date: string; venue: string; duration_s: number; performances: number; laughs: number; has_audio: boolean;
  analysis: { source: string; jokes_counted: number; new_material: number; crowd_work: number; best: any; laughs_per_min: number } | null;
  jokes: { perf: string; title: string; joke: string | null; kind: string | null; rating: string | null; rating_by: string; start_ms: number }[];
  // Requested by app
  show_date?: string | null; show_date_source?: string | null; description?: string | null; laugh_s_per_min?: number | null; room_heat?: number | null;
  audio_unreliable?: boolean | null; venue_id?: string | null;
};
