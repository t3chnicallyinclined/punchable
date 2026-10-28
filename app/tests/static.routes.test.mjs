// Static check: the route files under src/app are well-formed for expo-router (docs/ENGINEERING.md "Routes").
//  1. Every route file (every .ts/.tsx under src/app, layouts and +not-found included) has a default export: a file
//     without one is a blank screen on the phone and a warning nobody reads.
//  2. No `unstable_settings.initialRouteName`: deprecated for `anchor` in expo-router (docs.expo.dev/router/advanced/
//     router-settings). KNOWN_INITIAL_ROUTE_NAME is the ratchet: the files that still use it today; a new one fails, a
//     fixed one left in the list fails (delete it).
//  3. Route files stay thin: nothing under src/app exports a React component other than its default (a second
//     component export turns a route file into a shared module, and expo-router warns about non-route exports).
//   node --experimental-strip-types --no-warnings tests/static.routes.test.mjs
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const APP = resolve(fileURLToPath(new URL('../src/app', import.meta.url)));
const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const rel = (p) => relative(APP, p).replace(/\\/g, '/');
const fails = [];
const check = (c, m) => { if (!c) fails.push(m); };

/** Still on `initialRouteName`: none since 2026-10-28 (the three tab layouts moved to `anchor`). Stays empty. */
const KNOWN_INITIAL_ROUTE_NAME = new Set([]);
/** Named component exports that a route file is allowed to keep for now (the builder moves them under src/features). */
const KNOWN_EXTRA_EXPORTS = new Set(['(tabs)/(tonight)/index.tsx:SetBar', '(tabs)/(bits)/write.tsx:BitRow', '(tabs)/(sets)/sets.tsx:PaperCard']);

const files = walk(APP).filter((p) => /\.(ts|tsx)$/.test(p) && !/\.d\.ts$/.test(p));
const DEFAULT = /export\s+default\b|export\s*\{[^}]*\bdefault\b[^}]*\}/;
const INITIAL = /unstable_settings\s*=\s*\{[^}]*\binitialRouteName\b/;
const NAMED_COMPONENT = /^export\s+(?:const|function|class)\s+([A-Z]\w*)/gm;

const seenInitial = new Set();
const seenExtra = new Set();
for (const p of files) {
  const r = rel(p);
  const src = readFileSync(p, 'utf8');
  check(DEFAULT.test(src), `${r}: no default export (a route must export its screen as default)`);
  if (INITIAL.test(src)) {
    seenInitial.add(r);
    check(KNOWN_INITIAL_ROUTE_NAME.has(r), `${r}: unstable_settings.initialRouteName is deprecated; use { anchor: '<file>' }`);
  }
  let m;
  while ((m = NAMED_COMPONENT.exec(src))) {
    if (m[1] === 'ErrorBoundary') continue; // expo-router reads this named export from a route (docs.expo.dev/router/error-handling)
    const key = `${r}:${m[1]}`;
    seenExtra.add(key);
    check(KNOWN_EXTRA_EXPORTS.has(key), `${r}: exports component ${m[1]} from a route file; move it under src/lib (or src/features) and import it`);
  }
}
for (const k of KNOWN_INITIAL_ROUTE_NAME) check(seenInitial.has(k), `stale baseline: ${k} no longer uses initialRouteName; delete it from KNOWN_INITIAL_ROUTE_NAME`);
for (const k of KNOWN_EXTRA_EXPORTS) check(seenExtra.has(k), `stale baseline: ${k} is no longer exported from the route; delete it from KNOWN_EXTRA_EXPORTS`);

console.log(`${files.length} route files checked; ${seenInitial.size} still on initialRouteName (baseline ${KNOWN_INITIAL_ROUTE_NAME.size})`);
for (const f of fails) console.log('FAIL ' + f);
console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
