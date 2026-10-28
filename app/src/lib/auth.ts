// Sign-in for the hosted app (punchable.io): Keycloak realm `punchable`, authorization code + PKCE (S256).
//   web:    client pk-web,    redirect https://punchable.io/auth/callback  (expo-auth-session popup)
//   native: client pk-mobile, redirect punchable://auth/callback
// Tokens: the access token lives in memory only. The refresh token is kept in expo-secure-store on the phone and in
// sessionStorage on the web (gone when the tab closes). The access token is refreshed 60 s before it expires; the
// realm rotates refresh tokens, so each refresh stores the new one.
// Wiring (no change to the screens): importing this module once (the root layout: `import '@/lib/auth';`)
//   - registers the bearer with api.ts (setAuthHeaderProvider),
//   - sends the token as the first message on /api/live (never in the URL),
//   - sends him to /login?next=<the page he was on> on any 401 from the API.
// Session state for screens: useSession() (signed in? who? from the ID token's claims). signOut() ends the Keycloak
// session too (RP-initiated logout with id_token_hint, back to https://punchable.io/).
// Local dev (the API on 127.0.0.1) has no auth: all of this stays off, unless the tab was opened with ?auth=on
// (the e2e test uses it to see the signed-out behaviour; the switch can only turn sign-in ON, never off).
import { useEffect, useState } from 'react';
import * as AuthSession from 'expo-auth-session';
import * as WebBrowser from 'expo-web-browser';
import * as SecureStore from 'expo-secure-store';
import { router } from 'expo-router';
import { Platform } from 'react-native';
import { API_URL, setAuthHeaderProvider } from './api';
import { Profile, endSessionUrl, loginHref, originSlash, profileFrom } from './session';
import { retry } from './retry';

if (Platform.OS === 'web') WebBrowser.maybeCompleteAuthSession(); // web only: the popup's /auth/callback hands the code back to the opener

const env = (k: string) => (process.env as Record<string, string | undefined>)[k];
export const ISSUER = env('EXPO_PUBLIC_OIDC_ISSUER') || 'https://auth.punchable.io/realms/punchable';
export const CLIENT_ID = Platform.OS === 'web' ? env('EXPO_PUBLIC_OIDC_CLIENT_ID') || 'pk-web' : 'pk-mobile';
export const REDIRECT_URI =
  Platform.OS === 'web' ? env('EXPO_PUBLIC_OIDC_REDIRECT') || 'https://punchable.io/auth/callback' : 'punchable://auth/callback';
export const SCOPES = ['openid', 'profile', 'email'];
/** Where Keycloak sends the browser after sign-out: must be in the client's post.logout.redirect.uris. */
export const POST_LOGOUT_URI =
  Platform.OS === 'web' ? env('EXPO_PUBLIC_OIDC_POST_LOGOUT') || originSlash(REDIRECT_URI) : 'punchable://auth/logout';

/** The authorization-code + PKCE request every sign-in uses (login screen, "Reconnect Google"). */
export function authRequestConfig(extraParams?: Record<string, string>): AuthSession.AuthRequestConfig {
  return {
    clientId: CLIENT_ID,
    redirectUri: REDIRECT_URI,
    scopes: SCOPES,
    responseType: AuthSession.ResponseType.Code,
    usePKCE: true,
    codeChallengeMethod: AuthSession.CodeChallengeMethod.S256,
    ...(extraParams ? { extraParams } : {}),
  };
}

function sstore(): any {
  try {
    return (globalThis as any).sessionStorage ?? null;
  } catch {
    return null;
  }
}
function forcedOn(): boolean {
  try {
    const loc = (globalThis as any).location;
    const q = loc ? new URLSearchParams(loc.search).get('auth') : null;
    if (q === 'on') sstore()?.setItem('punchable.auth', 'on');
    return sstore()?.getItem('punchable.auth') === 'on';
  } catch {
    return false;
  }
}

/** Auth is on when the API is not a loopback dev server (or EXPO_PUBLIC_AUTH=1 forces it). */
function computeEnabled(): boolean {
  if (env('EXPO_PUBLIC_AUTH') === '1' || forcedOn()) return true;
  if (env('EXPO_PUBLIC_AUTH') === '0') return false;
  const base = API_URL || ((globalThis as any).location?.origin ?? '');
  return !/^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(base) && base !== '';
}
export const AUTH_ENABLED = computeEnabled();

const REFRESH_KEY = 'punchable.refresh';
const WAY_KEY = 'punchable.signin-way'; // 'google' | 'email': which button he used last on this device (a label only)
type Tokens = { access: string; expiresAt: number; refresh: string | null; idToken: string | null };
let tokens: Tokens | null = null;
// Has the stored refresh token been tried yet? Until then the app doesn't know whether he is signed in.
let checked = false;
let discovery: AuthSession.DiscoveryDocument | null = null;
let refreshing: Promise<string | null> | null = null;
const listeners = new Set<(signedIn: boolean) => void>();

// ---------------------------------------------------------------- refresh-token storage
// The keychain write is guarded: it never throws into the sign-in path (setTokens does not await it), it retries a
// transient keychain failure (lib/retry), and a lasting one is logged, not fatal: he signs in again next time.
// AFTER_FIRST_UNLOCK: the token stays readable while the phone is locked after its first unlock, so a recording that
// uploads in the background can still refresh (expo-secure-store: keychainAccessible / kSecAttrAccessible).
// Size: expo-secure-store enforces no limit; the keychain itself rejects nothing at the sizes Keycloak issues (a
// refresh token here is ~1 KB); a native error, if any, lands in the catch below.
const KEYCHAIN = { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK };
async function saveRefresh(rt: string | null) {
  try {
    if (Platform.OS === 'web') {
      const ss = (globalThis as any).sessionStorage;
      if (rt) ss?.setItem(REFRESH_KEY, rt);
      else ss?.removeItem(REFRESH_KEY);
    } else if (rt) await retry(() => SecureStore.setItemAsync(REFRESH_KEY, rt, KEYCHAIN));
    else await retry(() => SecureStore.deleteItemAsync(REFRESH_KEY));
  } catch (e: any) {
    console.warn('[auth] the refresh token was not stored; sign-in will not survive a relaunch:', e?.message || String(e));
  }
}
async function loadRefresh(): Promise<string | null> {
  try {
    if (Platform.OS === 'web') return (globalThis as any).sessionStorage?.getItem(REFRESH_KEY) ?? null;
    return await SecureStore.getItemAsync(REFRESH_KEY);
  } catch {
    return null;
  }
}

export async function getDiscovery(): Promise<AuthSession.DiscoveryDocument> {
  if (!discovery) discovery = await AuthSession.fetchDiscoveryAsync(ISSUER);
  return discovery;
}

function setTokens(r: AuthSession.TokenResponse | null) {
  if (!r) {
    tokens = null;
  } else {
    const ttl = r.expiresIn ?? 300;
    tokens = {
      access: r.accessToken,
      expiresAt: (r.issuedAt ?? Math.floor(Date.now() / 1000)) + ttl,
      refresh: r.refreshToken ?? tokens?.refresh ?? null,
      idToken: r.idToken ?? tokens?.idToken ?? null, // memory only: the sign-out hint and the profile's claims
    };
  }
  saveRefresh(tokens?.refresh ?? null);
  checked = true;
  listeners.forEach((f) => f(!!tokens));
}

async function refreshWith(rt: string): Promise<string | null> {
  try {
    const d = await getDiscovery();
    const r = await AuthSession.refreshAsync({ clientId: CLIENT_ID, refreshToken: rt, scopes: SCOPES }, d);
    setTokens(r);
    return r.accessToken;
  } catch {
    setTokens(null); // expired or reused (rotation): sign in again
    return null;
  }
}

/** A valid access token, refreshed when it has under 60 s left; null when signed out. */
export async function getAccessToken(): Promise<string | null> {
  if (!AUTH_ENABLED) return null;
  const now = Math.floor(Date.now() / 1000);
  if (tokens && tokens.expiresAt - now > 60) return tokens.access;
  if (refreshing) return refreshing;
  const rt = tokens?.refresh ?? (await loadRefresh());
  if (!rt) return null;
  refreshing = refreshWith(rt).finally(() => {
    refreshing = null;
  });
  return refreshing;
}

export function isSignedIn() {
  return !!tokens;
}

let rememberedWay: string | null = (() => {
  try {
    return Platform.OS === 'web' ? (globalThis as any).localStorage?.getItem(WAY_KEY) ?? null : null;
  } catch {
    return null;
  }
})();
/** Remember which sign-in button he used (shown as "Signed in with Google" when the token doesn't say). */
export function rememberWay(way: 'google' | 'email') {
  rememberedWay = way;
  try {
    if (Platform.OS === 'web') (globalThis as any).localStorage?.setItem(WAY_KEY, way);
  } catch {
    /* a label only */
  }
}

/** Who is signed in, from the ID token (name, email, picture, how). Null when signed out or auth is off. */
export function getProfile(): Profile | null {
  return tokens ? profileFrom(tokens.idToken, tokens.access, rememberedWay) : null;
}

let checking: Promise<void> | null = null;
function checkOnce(): Promise<void> {
  if (checked || !AUTH_ENABLED) return Promise.resolve();
  if (!checking) {
    const done = () => {
      checked = true;
      listeners.forEach((f) => f(!!tokens));
    };
    checking = getAccessToken().then(done, done);
  }
  return checking;
}

export type Session = { enabled: boolean; checking: boolean; signedIn: boolean; profile: Profile | null };
/** For screens: re-renders when he signs in or out. */
export function useSession(): Session {
  const read = (): Session => ({ enabled: AUTH_ENABLED, checking: AUTH_ENABLED && !checked, signedIn: !!tokens, profile: getProfile() });
  const [st, setSt] = useState<Session>(read);
  useEffect(() => {
    const off = onAuthChange(() => setSt(read()));
    checkOnce().then(() => setSt(read()));
    return off;
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return st;
}
export function onAuthChange(f: (signedIn: boolean) => void) {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}

/** The code from the login screen's AuthRequest -> tokens (the PKCE verifier proves it is the same client). */
export async function completeSignIn(code: string, codeVerifier: string | undefined) {
  const d = await getDiscovery();
  const r = await AuthSession.exchangeCodeAsync(
    { clientId: CLIENT_ID, code, redirectUri: REDIRECT_URI, extraParams: codeVerifier ? { code_verifier: codeVerifier } : undefined },
    d,
  );
  setTokens(r);
}

/**
 * Sign out of Punchable AND end the Keycloak session (otherwise the next "Continue with Google" signs him straight back
 * in without asking). Web: the browser goes to Keycloak's end_session endpoint with id_token_hint and comes back to
 * https://punchable.io/, signed out. Phone: the same URL in an auth session, back to punchable://auth/logout.
 * If Keycloak can't be reached, the refresh token is revoked instead; this device forgets it either way.
 */
export async function signOut(): Promise<void> {
  const idToken = tokens?.idToken ?? null;
  const rt = tokens?.refresh ?? (await loadRefresh());
  leaving = true;
  setTokens(null);
  if (!AUTH_ENABLED) {
    leaving = false;
    goLogin();
    return;
  }
  let d: AuthSession.DiscoveryDocument | null = null;
  try {
    d = await getDiscovery();
  } catch {
    d = null;
  }
  if (d?.endSessionEndpoint) {
    const url = endSessionUrl(d.endSessionEndpoint, { idToken, redirect: POST_LOGOUT_URI, clientId: CLIENT_ID });
    if (Platform.OS === 'web') {
      (globalThis as any).location.assign(url);
      return;
    }
    try {
      await WebBrowser.openAuthSessionAsync(url, POST_LOGOUT_URI);
    } catch {
      /* he closed the sheet: this device is signed out anyway */
    }
  } else if (rt && d?.revocationEndpoint) {
    try {
      await AuthSession.revokeAsync({ clientId: CLIENT_ID, token: rt }, d);
    } catch {
      /* already gone */
    }
  }
  leaving = false;
  goLogin();
}
/** True while the browser is on its way to Keycloak's sign-out page (nothing should redirect meanwhile). */
export let leaving = false;

let redirecting = false;
/** To /login, remembering the page he was on (web), so he lands back on it after signing in. */
export function goLogin() {
  if (redirecting || leaving) return;
  redirecting = true;
  setTimeout(() => {
    redirecting = false;
  }, 2000);
  let href = '/login';
  try {
    const loc = (globalThis as any).location;
    if (Platform.OS === 'web' && loc) href = loginHref(loc.pathname, loc.search);
  } catch {
    /* plain /login */
  }
  try {
    router.replace(href as any);
  } catch {
    /* the router is not mounted yet: the next 401 tries again */
  }
}

// ---------------------------------------------------------------- wiring into api.ts, fetch and the live socket
function apiOrigin(url: string) {
  const base = API_URL || ((globalThis as any).location?.origin ?? '');
  return url.startsWith(base + '/api/') || url.startsWith('/api/');
}

function install() {
  const g = globalThis as any;
  if (!AUTH_ENABLED || g.__punchableAuth) return;
  g.__punchableAuth = true;
  setAuthHeaderProvider(async (): Promise<Record<string, string>> => {
    const t = await getAccessToken();
    return t ? { Authorization: `Bearer ${t}` } : {};
  });
  const realFetch = g.fetch.bind(g);
  g.fetch = async (input: any, init?: any) => {
    const r = await realFetch(input, init);
    const url = typeof input === 'string' ? input : input?.url ?? '';
    if (r.status === 401 && apiOrigin(url) && !url.includes('/api/health')) {
      setTokens(null);
      goLogin();
    }
    return r;
  };
  const RealWS = g.WebSocket;
  if (RealWS) {
    class AuthedWebSocket extends RealWS {
      constructor(url: string, protocols?: string | string[]) {
        super(url, protocols);
        if (String(url).includes('/api/live')) {
          (this as any).addEventListener('open', async () => {
            const t = await getAccessToken();
            try {
              (this as any).send(JSON.stringify({ type: 'auth', token: t ?? '' }));
            } catch {
              /* closed meanwhile */
            }
          });
        }
      }
    }
    g.WebSocket = AuthedWebSocket;
  }
}
install();
