// Unit test for src/lib/crashlog.ts: an error becomes a readable, redacted, bounded record that survives a round trip.
//   node --experimental-strip-types --no-warnings tests/crashlog.test.mjs
import * as L from "../src/lib/crashlog.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1MSIsImVtYWlsIjoidEB4LmlvIn0.c2lnbmF0dXJlLXNpZ25hdHVyZQ";

// ---- an Error with a stack
const e = new Error("boom at mount");
const r = L.formatCrash(e, "boundary", { path: "/", build: "6" }, new Date("2026-10-28T01:02:03Z"));
check(r.v === 1 && r.source === "boundary" && r.name === "Error" && r.message === "boom at mount", "name, message, source");
check(r.at === "2026-10-28T01:02:03.000Z" && r.context.path === "/" && r.context.build === "6", "time and context kept");
check(r.stack.includes("boom at mount") || r.stack.includes("crashlog.test"), "the stack is kept");

// ---- anything thrown, never throws
check(L.formatCrash("a string", "fatal").message === "a string", "a thrown string");
check(L.formatCrash(null, "fatal").message === "null", "null");
check(L.formatCrash({ name: 42, message: { x: 1 } }, "fatal").name === "Error", "odd shapes fall back");

// ---- redaction
const leaky = new Error(`GET https://punchable.io/api/x Authorization: Bearer ${jwt} and id_token_hint=${jwt}&code=abc123&state=zzz`);
const rr = L.formatCrash(leaky, "fatal");
check(!rr.message.includes(jwt) && !rr.message.includes("abc123"), "bearer, jwt and code are gone");
check(rr.message.includes("Bearer [redacted]") && rr.message.includes("code=[redacted]"), "redaction markers are readable");
check(L.redact(`x ${jwt} y`) === "x [jwt redacted] y", "a bare jwt is redacted");
check(L.redact("nothing to see") === "nothing to see", "plain text untouched");
check(L.formatCrash(new Error("ok"), "fatal", { url: `https://a/b?token=${jwt}` }).context.url.includes("token=[redacted]"), "context values are redacted too");

// ---- bounds
const big = L.formatCrash({ name: "E", message: "m".repeat(5000), stack: "s".repeat(20000) }, "fatal");
check(big.message.length < L.MAX_MESSAGE + 40 && big.message.endsWith("more]"), `message capped (${big.message.length})`);
check(big.stack.length < L.MAX_STACK + 40, `stack capped (${big.stack.length})`);

// ---- round trip
const back = L.parseCrash(L.serializeCrash(r));
check(back && back.message === r.message && back.at === r.at && back.source === "boundary" && back.context.build === "6", "serialize -> parse");
check(L.parseCrash("") === null && L.parseCrash(null) === null && L.parseCrash("{not json") === null, "empty or broken -> null");
check(L.parseCrash(JSON.stringify({ v: 2, message: "x", at: "y" })) === null, "another version -> null");
check(L.parseCrash(JSON.stringify({ v: 1, message: "x", at: "y", source: "weird" })).source === "fatal", "unknown source -> fatal");

// ---- the text he reads
const t = L.crashText(r);
check(t.startsWith("Error: boom at mount\nat 2026-10-28T01:02:03.000Z (boundary)\npath: /\nbuild: 6"), "message first, then when, then context");
check(t.includes(r.stack.slice(0, 40)), "then the stack");

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
