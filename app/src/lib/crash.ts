// What a crash report may carry out of the app (pure: no Sentry import, so tests/crash.test.mjs can run it under node).
// crashReporting.ts wires these into @sentry/react-native's beforeSend / beforeBreadcrumb.
//
// The rule: his material never leaves through an error report. An event loses every request body and query string,
// every field at any depth named text / quote / transcript / query / title / ... (SENSITIVE_KEYS, plus any key that
// merely contains one of those words), every breadcrumb's message and free-form data, console breadcrumbs entirely,
// the user beyond an opaque id, and stack-frame locals. What survives is what locates a crash: the exception type and
// a bounded message with quoted literals blanked, the frames (file, function, line, column), the release, the
// environment, the device/OS context, and API breadcrumbs as ROUTE PATTERN + method + status.

export const SENSITIVE_KEYS = new Set([
  'text', 'quote', 'transcript', 'query', 'q', 'title', 'body', 'content', 'prompt', 'answer', 'lines', 'summary',
  'name', 'venue', 'filename', 'original_name', 'reason', 'note', 'notes', 'idea', 'data', 'payload', 'vars',
  'token', 'secret', 'password', 'authorization', 'cookie', 'cookies', 'set-cookie', 'api_key', 'apikey', 'key',
  'email', 'user', 'username', 'sub', 'ip_address', 'marks', 'segments', 'punch', 'evidence',
]);
export const KEY_HINT = /(text|quote|transcript|query|title|secret|token|password|cookie|auth)/i;
export const REDACTED = '[scrubbed]';
export const MAX_MESSAGE = 300;
const QUOTED = /'[^']*'|"[^"]*"/g;
const WORD = /^[a-z][a-z_-]{0,31}$/;
const MAX_SEGMENTS = 6;

export function sensitiveKey(k: string): boolean {
  const s = String(k).trim().toLowerCase().replace(/-/g, '_');
  return SENSITIVE_KEYS.has(s) || SENSITIVE_KEYS.has(s.replace(/_/g, '-')) || KEY_HINT.test(s);
}

/** `/api/joke/x:ab12?q=hi` -> `/api/joke/{id}`: the same rule as the API's metrics (api-dev/metrics.py). */
export function routePattern(path: string): string {
  let p = String(path || '/');
  p = p.replace(/^[a-z]+:\/\/[^/]+/i, ''); // a full URL: the host is not the route
  p = p.split('?')[0].split('#')[0];
  const segs = p.split('/').filter(Boolean);
  const out = segs.slice(0, MAX_SEGMENTS).map((s) => (WORD.test(s) ? s : '{id}'));
  if (segs.length > MAX_SEGMENTS) out.push('...');
  return out.length ? '/' + out.join('/') : '/';
}

/** An exception message or log line: first line only, quoted literals blanked, bounded. */
export function scrubText(s: unknown): string {
  const first = String(s ?? '').split('\n')[0];
  return first.replace(QUOTED, "'...'").slice(0, MAX_MESSAGE);
}

export function scrubValue(v: unknown, depth = 0): unknown {
  if (depth > 8) return REDACTED;
  if (Array.isArray(v)) return v.slice(0, 64).map((x) => scrubValue(x, depth + 1));
  if (v && typeof v === 'object') {
    const out: Record<string, unknown> = {};
    let i = 0;
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
      if (i++ >= 64) break;
      out[k] = sensitiveKey(k) ? REDACTED : scrubValue(x, depth + 1);
    }
    return out;
  }
  if (typeof v === 'string') return v.length <= 1000 ? v : v.slice(0, 1000) + '...';
  return v;
}

const FRAME_KEYS = ['filename', 'abs_path', 'function', 'lineno', 'colno', 'module', 'in_app', 'platform'];

function scrubFrames(st: any): { frames: any[] } | undefined {
  if (!st || !Array.isArray(st.frames)) return undefined;
  return {
    frames: st.frames.slice(-50).map((f: any) => {
      const out: Record<string, unknown> = {};
      for (const k of FRAME_KEYS) if (f && f[k] !== undefined) out[k] = f[k];
      return out; // no vars, no context_line, no pre_context / post_context
    }),
  };
}

export function scrubException(exc: any): { values: any[] } {
  const values: any[] = [];
  for (const v of (exc && Array.isArray(exc.values) ? exc.values : []).slice(0, 8)) {
    if (!v || typeof v !== 'object') continue;
    const item: Record<string, unknown> = { type: String(v.type ?? '').slice(0, 100), value: scrubText(v.value) };
    const st = scrubFrames(v.stacktrace);
    if (st) item.stacktrace = st;
    if (v.mechanism && typeof v.mechanism === 'object') item.mechanism = { type: v.mechanism.type, handled: v.mechanism.handled };
    values.push(item);
  }
  return { values };
}

/** beforeBreadcrumb. API/fetch breadcrumbs keep method, route pattern and status; console breadcrumbs are dropped
 *  (a console line may carry his words); everything else keeps its category and level only. */
export function scrubBreadcrumb(bc: any): any | null {
  if (!bc || typeof bc !== 'object') return null;
  const cat = String(bc.category ?? '');
  if (cat.startsWith('console')) return null;
  const out: Record<string, unknown> = {};
  for (const k of ['timestamp', 'type', 'category', 'level']) if (bc[k] !== undefined) out[k] = bc[k];
  const d = bc.data && typeof bc.data === 'object' ? bc.data : null;
  if (d) {
    const keep: Record<string, unknown> = {};
    const url = typeof d.url === 'string' ? d.url : typeof d.route === 'string' ? d.route : null;
    if (url) keep.route = routePattern(url);
    for (const k of ['method', 'status', 'status_code', 'reason', 'duration_ms']) {
      const v = d[k];
      if (typeof v === 'string' && k !== 'reason') keep[k] = v.slice(0, 16);
      else if (typeof v === 'number') keep[k] = v;
      else if (k === 'reason' && typeof v === 'string') keep[k] = v.slice(0, 32).replace(QUOTED, "'...'");
    }
    if (Object.keys(keep).length) out.data = keep;
  }
  return out;
}

/** The breadcrumb the API client adds for a failed request (api.ts -> crashReporting.ts). Status 0 = no response. */
export function apiBreadcrumb(method: string, path: string, status: number, reason?: string) {
  return {
    category: 'api',
    type: 'http',
    level: status >= 500 || status === 0 ? 'warning' : 'info',
    data: { route: routePattern(path), method: String(method).toUpperCase().slice(0, 8), status, ...(reason ? { reason: reason.slice(0, 32) } : {}) },
  };
}

const CONTEXT_KEYS = ['device', 'os', 'app', 'runtime', 'react_native_context', 'trace', 'culture'];

/** beforeSend. Returns the event to send: never null, a crash is always reported, just bare. */
export function scrubEvent(event: any): any {
  if (!event || typeof event !== 'object') return event;
  const out: Record<string, unknown> = {};
  for (const k of ['event_id', 'timestamp', 'platform', 'level', 'logger', 'release', 'dist', 'environment', 'transaction', 'sdk', 'type']) {
    if (event[k] !== undefined) out[k] = event[k];
  }
  if (event.message !== undefined) {
    const m = event.message;
    out.message = scrubText(typeof m === 'string' ? m : (m && m.formatted) || '');
  }
  if (event.exception) out.exception = scrubException(event.exception);
  if (event.request && typeof event.request === 'object') {
    const req: Record<string, unknown> = {};
    if (typeof event.request.method === 'string') req.method = event.request.method;
    if (typeof event.request.url === 'string') req.url = event.request.url.split('?')[0].split('#')[0];
    out.request = req; // headers, data, query_string, cookies: gone
  }
  const crumbs = Array.isArray(event.breadcrumbs) ? event.breadcrumbs : event.breadcrumbs && Array.isArray(event.breadcrumbs.values) ? event.breadcrumbs.values : null;
  if (crumbs) out.breadcrumbs = crumbs.slice(-30).map(scrubBreadcrumb).filter(Boolean);
  if (event.tags && typeof event.tags === 'object') {
    const tags: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(event.tags)) if (!sensitiveKey(k)) tags[k] = scrubValue(v);
    out.tags = tags;
  }
  if (event.contexts && typeof event.contexts === 'object') {
    const ctx: Record<string, unknown> = {};
    for (const k of CONTEXT_KEYS) if (event.contexts[k] && typeof event.contexts[k] === 'object') ctx[k] = scrubValue(event.contexts[k]);
    out.contexts = ctx;
  }
  if (event.extra && typeof event.extra === 'object') out.extra = scrubValue(event.extra);
  if (event.user && typeof event.user === 'object' && event.user.id != null) out.user = { id: String(event.user.id).slice(0, 64) };
  if (event.threads && Array.isArray(event.threads.values)) {
    out.threads = {
      values: event.threads.values.slice(0, 16).map((t: any) => ({ id: t.id, crashed: t.crashed, current: t.current, stacktrace: scrubFrames(t.stacktrace) })),
    };
  }
  return out;
}

/** Every key path in a (scrubbed) event; the test asserts no sensitive key survives with a value. */
export function flattenKeys(x: unknown, into: Set<string> = new Set()): Set<string> {
  if (Array.isArray(x)) for (const v of x) flattenKeys(v, into);
  else if (x && typeof x === 'object') {
    for (const [k, v] of Object.entries(x as Record<string, unknown>)) {
      into.add(k);
      flattenKeys(v, into);
    }
  }
  return into;
}

/** release = app version + build number: `punchable@0.1.0+12`; dist = the build number. */
export function releaseName(version?: string | null, build?: string | number | null): string {
  const v = String(version || '0.0.0').trim();
  const b = build == null || build === '' ? 'dev' : String(build).trim();
  return `punchable@${v}+${b}`;
}

/** production | preview | dev: EXPO_PUBLIC_ENV from eas.json's build profile; a development build is always dev. */
export function environmentName(envVar: string | undefined, isDev: boolean): 'production' | 'preview' | 'dev' {
  if (isDev) return 'dev';
  const e = String(envVar || '').toLowerCase();
  return e === 'preview' || e === 'dev' ? e : 'production';
}
