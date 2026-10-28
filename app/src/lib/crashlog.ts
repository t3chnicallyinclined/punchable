// The last crash, as a record a person can read off the phone (no React Native imports: tests/crashlog.test.mjs
// runs this in Node). Why: a TestFlight build has no red box; an uncaught JS error ends the process (RN 0.88
// RCTExceptionsManager.reportFatal -> RCTFatal throws an NSException in release) and all that is left is an .ips the
// owner must find and send. lib/crashReport.ts writes this record before the error goes on to React Native's own
// handler, so the NEXT launch can show it (the login screen, "Last crash"). Tokens are redacted: a stack or message
// may carry a URL with a bearer token or an id_token; the record is meant to be pasted into a chat.
export type CrashRecord = {
  v: 1;
  /** ISO time the error was caught on the phone. */
  at: string;
  /** 'fatal' = ErrorUtils global handler (the process was about to end); 'boundary' = caught by the root ErrorBoundary. */
  source: 'fatal' | 'boundary';
  name: string;
  message: string;
  stack: string;
  /** Where the app was (the pathname) and anything the caller adds (platform, build). */
  context: Record<string, string>;
};

export const MAX_MESSAGE = 2000;
export const MAX_STACK = 6000;

const BEARER = /Bearer\s+[A-Za-z0-9\-_.=]+/g;
const JWT = /eyJ[A-Za-z0-9\-_]{8,}\.[A-Za-z0-9\-_]{8,}\.[A-Za-z0-9\-_]{8,}/g;
const TOKEN_PARAM = /([?&](?:code|state|token|id_token_hint|refresh_token|access_token)=)[^&\s#]+/gi;
/** Strips bearer tokens, JWTs and token-like query parameters so the record can be shared. */
export function redact(s: string): string {
  return s.replace(BEARER, 'Bearer [redacted]').replace(JWT, '[jwt redacted]').replace(TOKEN_PARAM, '$1[redacted]');
}

const cut = (s: string, n: number) => (s.length > n ? s.slice(0, n) + ` ... [${s.length - n} more]` : s);

/** An Error (or anything thrown) -> a record. Never throws. */
export function formatCrash(err: unknown, source: CrashRecord['source'], context: Record<string, string> = {}, now: Date = new Date()): CrashRecord {
  let name = 'Error';
  let message = '';
  let stack = '';
  try {
    if (err && typeof err === 'object') {
      const e = err as { name?: unknown; message?: unknown; stack?: unknown };
      name = typeof e.name === 'string' && e.name ? e.name : 'Error';
      message = typeof e.message === 'string' ? e.message : String(err);
      stack = typeof e.stack === 'string' ? e.stack : '';
    } else {
      message = String(err);
    }
  } catch {
    message = '(unprintable error)';
  }
  const ctx: Record<string, string> = {};
  for (const k of Object.keys(context)) ctx[k] = redact(cut(String(context[k]), 300));
  return { v: 1, at: now.toISOString(), source, name: cut(redact(name), 100), message: cut(redact(message), MAX_MESSAGE), stack: cut(redact(stack), MAX_STACK), context: ctx };
}

export function serializeCrash(r: CrashRecord): string {
  return JSON.stringify(r);
}

/** The stored string -> a record, or null when it is not one (an old or damaged file is ignored, never thrown on). */
export function parseCrash(raw: string | null | undefined): CrashRecord | null {
  if (!raw) return null;
  try {
    const j = JSON.parse(raw);
    if (!j || typeof j !== 'object' || j.v !== 1 || typeof j.message !== 'string' || typeof j.at !== 'string') return null;
    return {
      v: 1, at: j.at, source: j.source === 'boundary' ? 'boundary' : 'fatal', name: typeof j.name === 'string' ? j.name : 'Error', message: j.message,
      stack: typeof j.stack === 'string' ? j.stack : '', context: j.context && typeof j.context === 'object' ? j.context : {},
    };
  } catch {
    return null;
  }
}

/** The text he reads off the phone or shares: the message first, when and where, then the stack. */
export function crashText(r: CrashRecord): string {
  const ctx = Object.keys(r.context).map((k) => `${k}: ${r.context[k]}`).join('\n');
  return [`${r.name}: ${r.message}`, `at ${r.at} (${r.source})`, ctx, '', r.stack].join('\n').replace(/\n{3,}/g, '\n\n').trim();
}
