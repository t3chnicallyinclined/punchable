// The token state machine (no React Native imports: tests/tokens.test.mjs drives it with a fake Keycloak and a fake
// keychain). lib/auth.ts owns the real pieces (expo-auth-session refreshAsync, expo-secure-store) and hands them in.
//
// Rules, each one a scenario the tests play:
//   1. Durable before used: a new token set is written to the store BEFORE it is published to memory and listeners,
//      so no caller holds a token whose refresh token is not on disk (a JS reload between the two would otherwise
//      leave the OLD, already-rotated refresh token on disk: Keycloak's revokeRefreshToken=true makes it invalid_grant).
//   2. One refresh in flight: getAccessToken() (HTTP headers, the live socket, uploads) and handle401() all join the
//      same promise, so two paths never race two refreshes with one rotating token.
//   3. Only Keycloak's word signs him out: a TokenError with error=invalid_grant (the token endpoint's 400 for a
//      revoked, reused or expired refresh token). A network failure, a 5xx, an unparsable body: retry with backoff,
//      then keep the session (the stale access token goes out; the API decides) and try again next time.
//   4. Rotation grace: on invalid_grant, if the store holds a different refresh token than the one tried (another
//      context refreshed and saved), try that once before signing out.
export type TokenSet = { access: string; expiresAt: number; refresh: string | null; idToken: string | null };
/** What the token endpoint returns (expo-auth-session's TokenResponse has these fields). */
export type TokenReply = { accessToken?: string | null; expiresIn?: number | null; issuedAt?: number | null; refreshToken?: string | null; idToken?: string | null };
export type Store = { load: () => Promise<string | null>; save: (rt: string | null) => Promise<void> };
export type Deps = {
  /** refresh_token grant for `rt`; throws what expo-auth-session throws (TokenError with .code, or a fetch TypeError). */
  refresh: (rt: string) => Promise<TokenReply>;
  store: Store;
  /** Seconds since the epoch (tests pin it). */
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  /** Extra tries after a transient failure (default 2: three tries, 0.5 s then 1 s apart). */
  retries?: number;
  log?: (msg: string) => void;
};

/** Refresh when the access token has under this many seconds left. */
export const SKEW_S = 60;
const DEFAULT_TTL_S = 300;
const FIRST_DELAY_MS = 500;

/** Keycloak said the refresh token is dead: invalid_grant (revoked by rotation, reused, session expired). */
export function isRevoked(e: unknown): boolean {
  if (!e || typeof e !== 'object') return false;
  // expo-auth-session's TokenError (ResponseError) keeps the endpoint's body in .params and the error in .code
  const x = e as { code?: unknown; error?: unknown; params?: { error?: unknown } };
  const code = typeof x.code === 'string' ? x.code : typeof x.error === 'string' ? x.error : typeof x.params?.error === 'string' ? x.params.error : '';
  return code === 'invalid_grant';
}

type Attempt = { kind: 'ok'; access: string } | { kind: 'revoked'; error: unknown } | { kind: 'transient'; error: unknown };

export class Tokens {
  tokens: TokenSet | null = null;
  /** Has the stored refresh token been tried once? Until then the app does not know whether he is signed in. */
  checked = false;
  private inflight: Promise<string | null> | null = null;
  private listeners = new Set<(signedIn: boolean) => void>();
  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly retries: number;
  private readonly log: (msg: string) => void;
  private readonly d: Deps;

  constructor(d: Deps) {
    this.d = d; // a plain field: Node's --experimental-strip-types (the tests) rejects a TS parameter property
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    this.sleep = d.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
    this.retries = d.retries ?? 2;
    this.log = d.log ?? (() => {});
  }

  get signedIn(): boolean {
    return !!this.tokens;
  }
  onChange(f: (signedIn: boolean) => void): () => void {
    this.listeners.add(f);
    return () => {
      this.listeners.delete(f);
    };
  }
  private publish(next: TokenSet | null) {
    this.tokens = next;
    this.checked = true;
    this.listeners.forEach((f) => f(!!next));
  }
  private async persist(rt: string | null) {
    try {
      await this.d.store.save(rt);
    } catch (e: any) {
      this.log(`refresh token not stored: ${e?.message || String(e)}`); // best effort: the session is still good in memory
    }
  }

  /** A token reply (code exchange or refresh) -> stored, then live. Throws on a reply without an access token. */
  async accept(r: TokenReply): Promise<string> {
    if (!r.accessToken) throw new Error('token reply without access_token');
    const next: TokenSet = {
      access: r.accessToken,
      expiresAt: (r.issuedAt ?? this.now()) + (r.expiresIn ?? DEFAULT_TTL_S),
      refresh: r.refreshToken ?? this.tokens?.refresh ?? null,
      idToken: r.idToken ?? this.tokens?.idToken ?? null,
    };
    await this.persist(next.refresh); // rule 1: on disk first
    this.publish(next);
    return next.access;
  }

  /** Signed out on this device (the caller ends the Keycloak session). */
  async clear(): Promise<void> {
    await this.persist(null);
    this.publish(null);
  }

  /** A valid access token; null when signed out. A network problem keeps the session and returns the stale token. */
  getAccessToken(): Promise<string | null> {
    const t = this.tokens;
    if (t && t.expiresAt - this.now() > SKEW_S) return Promise.resolve(t.access);
    return this.refreshNow();
  }

  /** The API answered 401 to a token the clock still liked: refresh now (shared). Resolves to "still signed in?". */
  async handle401(): Promise<boolean> {
    await this.refreshNow();
    return this.signedIn;
  }

  private refreshNow(): Promise<string | null> {
    if (!this.inflight) {
      this.inflight = this.run().finally(() => {
        this.inflight = null;
      });
    }
    return this.inflight;
  }

  private async run(): Promise<string | null> {
    const rt = this.tokens?.refresh ?? (await this.safeLoad());
    if (!rt) {
      if (!this.checked) this.publish(null); // nothing stored: signed out, and now we know
      return null;
    }
    const first = await this.attempt(rt);
    if (first.kind === 'ok') return first.access;
    if (first.kind === 'transient') return this.keep(first.error);
    // revoked: rule 4, the store may hold a newer token than memory
    const stored = await this.safeLoad();
    if (stored && stored !== rt) {
      const again = await this.attempt(stored);
      if (again.kind === 'ok') return again.access;
      if (again.kind === 'transient') return this.keep(again.error);
    }
    this.log('refresh token revoked (invalid_grant): signed out');
    await this.clear();
    return null;
  }

  /** Transient trouble: the session stays; the stale access token (if any) is what we have. */
  private keep(error: unknown): string | null {
    this.log(`token refresh failed, keeping the session: ${(error as any)?.message || String(error)}`);
    if (!this.checked) this.checked = true; // cold start offline: known-unknown; the stored token stays for next time
    return this.tokens?.access ?? null;
  }

  private async safeLoad(): Promise<string | null> {
    try {
      return await this.d.store.load();
    } catch {
      return null;
    }
  }

  private async attempt(rt: string): Promise<Attempt> {
    let delay = FIRST_DELAY_MS;
    for (let i = 0; ; i++) {
      try {
        const r = await this.d.refresh(rt);
        return { kind: 'ok', access: await this.accept(r) };
      } catch (e) {
        if (isRevoked(e)) return { kind: 'revoked', error: e };
        if (i >= this.retries) return { kind: 'transient', error: e };
        await this.sleep(delay);
        delay *= 2;
      }
    }
  }
}
