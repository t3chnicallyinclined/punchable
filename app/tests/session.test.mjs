// Unit test for src/lib/session.ts: who is signed in (claims), where he goes after signing in (no open redirect),
// and the Keycloak sign-out URL.   node --experimental-strip-types --no-warnings tests/session.test.mjs
import * as S from "../src/lib/session.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const b64u = (o) => Buffer.from(JSON.stringify(o)).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const jwt = (o) => `${b64u({ alg: "RS256" })}.${b64u(o)}.sig`;

// ---- claims -> profile
const id = jwt({ sub: "u1", name: "Sam Rivera", email: "owner@example.com", picture: "https://lh3.googleusercontent.com/a/x=s96-c", identity_provider: "google" });
const p = S.profileFrom(id, null, null);
check(p.name === "Sam Rivera" && p.email === "owner@example.com" && p.initials === "SR", "name, email, initials from the ID token");
check(p.picture === "https://lh3.googleusercontent.com/a/x=s96-c" && p.via === "google", "photo and 'Signed in with Google' from the token");
const p2 = S.profileFrom(jwt({ sub: "u1", given_name: "Sam", family_name: "Doe", email: "t@x.io" }), null, "google");
check(p2.name === "Sam Doe" && p2.via === "google" && p2.picture === null, "no idp claim: falls back to the remembered button; no picture -> initials");
check(S.profileFrom(jwt({ email: "t@x.io", picture: "javascript:alert(1)" }), null, null).picture === null, "a non-https picture is ignored");
check(S.profileFrom(jwt({ email: "t@x.io", identity_provider: "github" }), null, "google").via === "email", "the token's provider wins over the remembered one");
check(S.profileFrom(null, null, "google") === null && S.jwtClaims("not-a-jwt") === null, "no token -> no profile");
check(S.profileFrom(jwt({ name: "Zoë Ünal" }), null, null).initials === "ZÜ", "UTF-8 names decode");

// ---- where he goes after signing in
const ok = { "/joke/joke%3Aairport-bit": "/joke/joke:airport-bit", "/board/sb:robot-vacuum?x=1": "/board/sb:robot-vacuum?x=1", "/": "/" };
for (const [raw, want] of Object.entries(ok)) check(S.safeNext(raw) === want, `next ${raw} -> ${want}`);
const bad = ["https://evil.com", "//evil.com", "/\\evil.com", "%2F%2Fevil.com", "javascript:alert(1)", "/login", "/login?next=/x", "/auth/callback", "evil", "", null, 5, "/x\nSet-Cookie", "/%E0%A4%A"];
for (const raw of bad) check(S.safeNext(raw) === "/", `next ${JSON.stringify(raw)} -> / (refused)`);
check(S.loginHref("/joke/joke:airport-bit", "?api=http://127.0.0.1:9&auth=on&tab=connect") === "/login?next=" + encodeURIComponent("/joke/joke:airport-bit?tab=connect"), "loginHref keeps the page and its query, drops the dev switches");
check(S.loginHref("/", "") === "/login" && S.loginHref("/login", "?next=/x") === "/login", "loginHref: Today or login -> plain /login");
check(S.isPublicPath("/login") && S.isPublicPath("/privacy") && S.isPublicPath("/privacy.html") && !S.isPublicPath("/") && !S.isPublicPath("/joke/x") && !S.isPublicPath("/loginx"), "public pages: login, callback, privacy, terms only");

// ---- sign-out URL
const u = new URL(S.endSessionUrl("https://auth.punchable.io/realms/punchable/protocol/openid-connect/logout", { idToken: id, redirect: "https://punchable.io/", clientId: "pk-web" }));
check(u.searchParams.get("id_token_hint") === id && u.searchParams.get("post_logout_redirect_uri") === "https://punchable.io/" && u.searchParams.get("client_id") === "pk-web", "end_session with id_token_hint, post_logout_redirect_uri=https://punchable.io/, client_id");
check(!new URL(S.endSessionUrl("https://kc/logout", { idToken: null, redirect: "https://punchable.io/", clientId: "pk-web" })).searchParams.has("id_token_hint"), "no ID token: client_id alone");
check(S.originSlash("https://punchable.io/auth/callback") === "https://punchable.io/", "post-logout redirect = the site root");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
