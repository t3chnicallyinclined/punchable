// Pure session helpers (no React Native imports, so tests/session.test.mjs runs them in Node):
// who is signed in (from the ID token's claims), where to go after sign-in, and the Keycloak sign-out URL.

export type Profile = {
  sub: string | null;
  name: string | null;
  email: string | null;
  picture: string | null;
  /** 'google' when the token says so (realm mapper `identity_provider`), else what this device remembers. */
  via: 'google' | 'email' | null;
  initials: string;
};

/** The payload of a JWT, without checking it (the API checks tokens; this only labels the profile page). */
export function jwtClaims(token: string | null | undefined): Record<string, any> | null {
  if (!token) return null;
  const part = token.split('.')[1];
  if (!part) return null;
  try {
    const b64 = part.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(part.length / 4) * 4, '=');
    const bin = (globalThis as any).atob ? (globalThis as any).atob(b64) : '';
    if (!bin) return null;
    const bytes = Uint8Array.from(bin, (c: string) => c.charCodeAt(0));
    const j = JSON.parse(new TextDecoder().decode(bytes));
    return j && typeof j === 'object' ? j : null;
  } catch {
    return null;
  }
}

export function initialsOf(name: string | null, email: string | null): string {
  const n = (name || '').trim();
  if (n) {
    const w = n.split(/\s+/).filter(Boolean);
    return ((w[0]?.[0] || '') + (w.length > 1 ? w[w.length - 1][0] : '')).toUpperCase();
  }
  return (email || '').trim().slice(0, 1).toUpperCase();
}

/** Only an https picture URL is shown (Google's lh3.googleusercontent.com); anything else is ignored. */
function safePicture(p: unknown): string | null {
  return typeof p === 'string' && /^https:\/\/[^\s"'<>]+$/.test(p) && p.length < 2048 ? p : null;
}

export function profileFrom(idToken: string | null | undefined, accessToken: string | null | undefined, rememberedVia: string | null): Profile | null {
  const c = { ...(jwtClaims(accessToken) || {}), ...(jwtClaims(idToken) || {}) };
  if (!Object.keys(c).length) return null;
  const name = (typeof c.name === 'string' && c.name) || [c.given_name, c.family_name].filter((x) => typeof x === 'string' && x).join(' ') || null;
  const email = typeof c.email === 'string' ? c.email : null;
  const idp = typeof c.identity_provider === 'string' ? c.identity_provider : null;
  const via = idp === 'google' ? 'google' : idp ? 'email' : rememberedVia === 'google' || rememberedVia === 'email' ? rememberedVia : null;
  return { sub: typeof c.sub === 'string' ? c.sub : null, name, email, picture: safePicture(c.picture), via, initials: initialsOf(name, email) };
}

// ---------------------------------------------------------------- where he goes after signing in
/** Routes a signed-out visitor may see. Everything else sends him to /login?next=<that route>. */
export const PUBLIC_PATHS = ['/login', '/auth/callback', '/auth/logout', '/privacy', '/terms'];
export const isPublicPath = (p: string) => PUBLIC_PATHS.some((x) => p === x || p.startsWith(x + '/') || p.startsWith(x + '.'));

/**
 * A `next` that can only be a path inside this app: starts with one "/", no scheme, no "//host", no backslash,
 * no control characters, and never the login or callback pages (that would loop). Anything else is "/".
 */
export function safeNext(raw: unknown): string {
  if (typeof raw !== 'string' || !raw) return '/';
  let p = raw;
  try {
    p = decodeURIComponent(raw);
  } catch {
    return '/';
  }
  if (p.length > 1000) return '/';
  if (!p.startsWith('/') || p.startsWith('//') || p.includes('\\') || /[\u0000-\u001f\u007f]/.test(p)) return '/';
  if (/^\/[a-z][a-z0-9+.-]*:/i.test(p)) return '/';
  const path = p.split(/[?#]/)[0];
  if (path === '/login' || path.startsWith('/auth/')) return '/';
  return p;
}

/** /login?next=… for the page he asked for (the dev-only ?api= / ?auth= switches are dropped). */
export function loginHref(pathname: string, search = ''): string {
  const q = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  q.delete('api');
  q.delete('auth');
  const qs = q.toString();
  const next = safeNext(pathname + (qs ? '?' + qs : ''));
  return next === '/' ? '/login' : '/login?next=' + encodeURIComponent(next);
}

// ---------------------------------------------------------------- sign-out
/**
 * Keycloak's RP-initiated logout (OIDC): ends the SSO session, then sends the browser to `redirect`, which must be in
 * the client's post.logout.redirect.uris (pk-web: https://punchable.io/). With id_token_hint Keycloak skips its
 * "Do you want to log out?" page; client_id is sent too, so it still works when no ID token is at hand.
 */
export function endSessionUrl(endpoint: string, opts: { idToken?: string | null; redirect: string; clientId: string }): string {
  const q = new URLSearchParams();
  if (opts.idToken) q.set('id_token_hint', opts.idToken);
  q.set('post_logout_redirect_uri', opts.redirect);
  q.set('client_id', opts.clientId);
  return endpoint + (endpoint.includes('?') ? '&' : '?') + q.toString();
}

/** https://punchable.io/auth/callback -> https://punchable.io/ (the post-logout redirect the realm allows). */
export function originSlash(redirectUri: string): string {
  const m = /^(https?:\/\/[^/]+)/.exec(redirectUri);
  return m ? m[1] + '/' : redirectUri;
}
