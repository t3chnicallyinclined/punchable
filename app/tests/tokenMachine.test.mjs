// Unit test for src/lib/tokenMachine.ts: the refresh state machine against a fake Keycloak and a fake keychain.
//   node --experimental-strip-types --no-warnings tests/tokenMachine.test.mjs
import { Tokens, SKEW_S, isRevoked } from "../src/lib/tokenMachine.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const tick = () => new Promise((r) => setTimeout(r, 0));

class TokenError extends Error { constructor(code) { super(code); this.code = code; } }
const netErr = () => new TypeError("Network request failed");
const NOW = 1_800_000_000;

/** A fake keychain shared between machines (a reload = a new machine over the same store). */
function store(initial = null) {
  const s = { value: initial, saves: [], events: [], slow: null, load: async () => s.value, save: async (rt) => { if (s.slow) await s.slow(); s.value = rt; s.saves.push(rt); s.events.push("save:" + rt); } };
  return s;
}
/** A fake token endpoint: `plan` maps a refresh token to a reply or an Error (thrown); calls are recorded. */
function keycloak(plan) {
  const k = { calls: [], refresh: async (rt) => { k.calls.push(rt); const r = plan(rt, k.calls.length); if (r instanceof Error) throw r; return r; } };
  return k;
}
const reply = (n, ttl = 300) => ({ accessToken: "access" + n, refreshToken: "rt" + n, idToken: "id" + n, expiresIn: ttl, issuedAt: NOW });
const machine = (k, s, extra = {}) => {
  const sleeps = [];
  const m = new Tokens({ refresh: k.refresh, store: s, now: () => NOW, sleep: async (ms) => { sleeps.push(ms); }, ...extra });
  const changes = [];
  m.onChange((b) => changes.push(b));
  return { m, sleeps, changes };
};

// ---- 1. durable before used: the store has the new refresh token before the caller gets the access token
{
  const s = store("rt0");
  const k = keycloak(() => reply(1));
  const { m, changes } = machine(k, s);
  const p = m.getAccessToken().then((t) => { s.events.push("resolve:" + t); return t; });
  const t = await p;
  check(t === "access1" && s.value === "rt1", "cold start: the stored token is refreshed, the new one stored");
  check(s.events.join(",") === "save:rt1,resolve:access1", "the save happens before the promise resolves");
  check(changes.join(",") === "true" && m.checked, "listeners hear signed-in once, after the save");
}
{
  // after sign-in, an expiring token: memory keeps the OLD set until the new refresh token is on disk
  const s = store("rtA");
  let during = null;
  const k = keycloak(() => reply(2));
  const { m } = machine(k, s, { now: () => NOW + 300 - 10 });
  await m.accept({ accessToken: "accessA", refreshToken: "rtA", expiresIn: 300, issuedAt: NOW });
  s.slow = async () => { during = m.tokens.access; };
  const t = await m.getAccessToken();
  check(during === "accessA" && t === "access2" && s.value === "rt2", "memory still holds the old set while the new refresh token is being written");
}

// ---- 2. reload between refresh and save (the coordinator's scenario) with the rotation grace
{
  const s = store("rtA");
  let release;
  s.slow = () => new Promise((r) => { release = r; }); // machine 1's save is in flight when the reload happens
  const k = keycloak((rt) => (rt === "rtA" ? reply(2) : rt === "rt2" ? reply(3) : new TokenError("invalid_grant")));
  const one = machine(k, s);
  const p1 = one.m.getAccessToken();
  await tick(); await tick();
  check(s.value === "rtA" && k.calls.join(",") === "rtA", "machine 1 refreshed rtA -> rt2 but has not finished writing it");
  // the reload: a new machine over the same store. It loads rtA (still on disk), Keycloak rejects it (rotated away);
  // machine 1's write lands meanwhile, so the grace re-read finds rt2.
  const k2 = { calls: [], refresh: async (rt) => {
    k2.calls.push(rt);
    if (rt === "rtA") { s.slow = null; release(); await p1; throw new TokenError("invalid_grant"); }
    if (rt === "rt2") return reply(3);
    throw new TokenError("invalid_grant");
  } };
  const two = machine(k2, s);
  const t2 = await two.m.getAccessToken();
  check(k2.calls.join(",") === "rtA,rt2" && t2 === "access3", "machine 2: rtA is invalid_grant, the store now holds rt2 -> tried once -> signed in (grace)");
  check(two.changes.join(",") === "true" && s.value === "rt3", "no sign-out; the newest token is stored");
}

// ---- 3. concurrent refreshers: HTTP headers, the socket, uploads and a 401 all join one refresh
{
  const s = store("rt0");
  const k = keycloak(() => reply(1));
  const { m } = machine(k, s);
  const rs = await Promise.all([m.getAccessToken(), m.getAccessToken(), m.handle401(), m.getAccessToken(), m.handle401()]);
  check(k.calls.length === 1, `one refresh for five callers (${k.calls.length})`);
  check(rs[0] === "access1" && rs[2] === true && rs[4] === true, "every caller gets the result");
  const again = await m.getAccessToken();
  check(again === "access1" && k.calls.length === 1, "a valid token is served from memory");
}

// ---- 4. network blip: retry with backoff, no sign-out
{
  const s = store("rt0");
  const k = keycloak((rt, n) => (n < 3 ? netErr() : reply(1)));
  const { m, sleeps, changes } = machine(k, s);
  const t = await m.getAccessToken();
  check(t === "access1" && k.calls.length === 3 && sleeps.join(",") === "500,1000", "two failures, 0.5 s then 1 s, then the token");
  check(!changes.includes(false) && s.value === "rt1", "never signed out");
}
{
  // the network stays down: the session is kept, the stale token goes out, the stored token is untouched
  const s = store("rtA");
  const k = keycloak(() => netErr());
  const { m, changes } = machine(k, s, { now: () => NOW + 400 });
  await m.accept({ accessToken: "accessA", refreshToken: "rtA", expiresIn: 300, issuedAt: NOW });
  const t = await m.getAccessToken();
  check(t === "accessA" && m.signedIn && k.calls.length === 3, "expired access, no network: the stale token, still signed in, three tries");
  check(!changes.includes(false) && s.value === "rtA" && s.saves.length === 1, "the stored refresh token was not cleared");
  check((await m.handle401()) === true, "a 401 during a network failure does not sign him out either");
}
{
  // a 5xx or an HTML body shows up as a parse/other error: transient, not a sign-out
  const s = store("rt0");
  const k = keycloak((rt, n) => (n === 1 ? new SyntaxError("JSON Parse error: Unexpected token <") : reply(1)));
  const { m } = machine(k, s);
  check((await m.getAccessToken()) === "access1", "a bad body is retried");
}

// ---- 5. the real revoke: invalid_grant with nothing newer stored
{
  const s = store("rtA");
  const k = keycloak(() => new TokenError("invalid_grant"));
  const { m, changes, sleeps } = machine(k, s, { now: () => NOW + 400 });
  await m.accept({ accessToken: "accessA", refreshToken: "rtA", expiresIn: 300, issuedAt: NOW });
  const t = await m.getAccessToken();
  check(t === null && !m.signedIn && s.value === null, "invalid_grant: signed out, the store cleared");
  check(changes.join(",") === "true,false" && k.calls.length === 1 && sleeps.length === 0, "one call, no retry, listeners hear signed-out once");
  check((await m.getAccessToken()) === null && k.calls.length === 1, "afterwards: null without calling Keycloak");
  check((await m.handle401()) === false, "a 401 now reports signed out");
}

// ---- 6. a 401 on a clock-valid token forces one refresh
{
  const s = store("rtA");
  const k = keycloak(() => reply(2));
  const { m } = machine(k, s);
  await m.accept({ accessToken: "accessA", refreshToken: "rtA", expiresIn: 300, issuedAt: NOW });
  check((await m.getAccessToken()) === "accessA" && k.calls.length === 0, "the token is valid by the clock");
  const [a, b] = await Promise.all([m.handle401(), m.handle401()]);
  check(a === true && b === true && k.calls.length === 1 && m.tokens.access === "access2", "two 401s -> one forced refresh -> a new token");
}

// ---- 7. accept() edge cases and classification
{
  const s = store(null);
  const { m, changes } = machine(keycloak(() => reply(1)), s);
  await m.accept({ accessToken: "x", refreshToken: "r1", idToken: "i1", expiresIn: 300, issuedAt: NOW });
  await m.accept({ accessToken: "y", expiresIn: 300, issuedAt: NOW });
  check(m.tokens.refresh === "r1" && m.tokens.idToken === "i1" && s.value === "r1", "a reply without refresh/id token keeps the old ones");
  let err = null;
  try { await m.accept({ expiresIn: 300 }); } catch (e) { err = e; }
  check(err && m.tokens.access === "y", "a reply without access_token throws and changes nothing");
  check(changes.join(",") === "true,true", "listeners per accept");
  check((await m.getAccessToken()) === "y", `valid until ${SKEW_S} s before expiry`);
  const empty = machine(keycloak(() => reply(1)), store(null));
  check((await empty.m.getAccessToken()) === null && empty.m.checked && empty.changes.join(",") === "false", "nothing stored: signed out and checked");
  check(isRevoked(new TokenError("invalid_grant")) && isRevoked({ error: "invalid_grant" }) && isRevoked({ params: { error: "invalid_grant", error_description: "Token is not active" } }) && !isRevoked(netErr()) && !isRevoked(new TokenError("invalid_client")) && !isRevoked(null), "isRevoked: .code, .error, .params.error (expo-auth-session's shape); never a network error");
}

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
