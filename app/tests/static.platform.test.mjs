// Static check: one mechanism per platform decision, and no logging in release (docs/ENGINEERING.md).
//  1. `console.log` / `console.debug` / `console.info` only inside a `__DEV__` guard: in a bundled app every console
//     call is a JS-thread stall (reactnative.dev/docs/performance "Using console.log statements"), and the release
//     bundle strips `__DEV__` blocks. (`console.warn` / `console.error` stay: they are what the crash reporter hooks.)
//  2. A module that is split by file extension (`X.tsx` + `X.native.tsx`) does not ALSO branch on
//     `Platform.OS === 'ios'`: pick the file split, or `Platform.select`, not both in the same pair. In the web twin such
//     a branch is dead code; in the `.native` twin it is an iOS/Android fork that belongs in `X.ios.tsx`.
//     KNOWN_PLATFORM_BRANCHES is the ratchet (the files that do it today; shrink it, never grow it).
//   node --experimental-strip-types --no-warnings tests/static.platform.test.mjs
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = resolve(fileURLToPath(new URL('../src', import.meta.url)));
const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const rel = (p) => relative(SRC, p).replace(/\\/g, '/');
const fails = [];
const check = (c, m) => { if (!c) fails.push(m); };

/** Split files that still branch on Platform.OS === 'ios' (2026-10-28): the KeyboardAvoidingView fork in the screen
 *  shell (both twins), the Liquid Glass probe, and the Android fallbacks in Icon.native / RowMenu.native (those two
 *  become Icon.ios.tsx / RowMenu.ios.tsx with the plain file serving Android and the web). Delete each line as it goes. */
const KNOWN_PLATFORM_BRANCHES = new Set(['lib/Screen.tsx', 'lib/Screen.native.tsx', 'lib/accessory.native.ts', 'lib/Icon.native.tsx', 'lib/RowMenu.native.tsx']);

const files = walk(SRC).filter((p) => /\.(ts|tsx)$/.test(p) && !/\.d\.ts$/.test(p));

// 1. console.log outside __DEV__
const LOG = /\bconsole\.(log|debug|info)\s*\(/;
for (const p of files) {
  const lines = readFileSync(p, 'utf8').split(/\r?\n/);
  lines.forEach((line, i) => {
    if (!LOG.test(line) || /^\s*\/\//.test(line)) return;
    const window = lines.slice(Math.max(0, i - 4), i + 1).join('\n');
    check(/__DEV__/.test(window), `${rel(p)}:${i + 1}: console.log outside a __DEV__ guard`);
  });
}

// 2. Platform.OS === 'ios' in a file-split pair
const IOS = /Platform\.OS\s*(===|==|!==|!=)\s*['"]ios['"]|Platform\.select\(\s*\{[^}]*\bios\b/;
const hasTwin = (p) => {
  const m = basename(p).match(/^(.*?)(\.(ios|android|native|web))?\.(tsx?)$/);
  if (!m) return false;
  const base = join(dirname(p), m[1]);
  const variants = m[3] ? ['', 'native', 'web', 'ios', 'android'].filter((v) => v !== m[3]) : ['native', 'web', 'ios', 'android'];
  return variants.some((v) => ['tsx', 'ts'].some((e) => existsSync(`${base}${v ? '.' + v : ''}.${e}`)));
};
const seen = new Set();
for (const p of files) {
  if (!hasTwin(p)) continue;
  if (!IOS.test(readFileSync(p, 'utf8'))) continue;
  const r = rel(p);
  seen.add(r);
  check(KNOWN_PLATFORM_BRANCHES.has(r), `${r}: a file-split module also branches on Platform.OS === 'ios'; use the .ios/.native split or Platform.select in ONE place`);
}
for (const k of KNOWN_PLATFORM_BRANCHES) check(seen.has(k), `stale baseline: ${k} no longer branches on ios; delete it from KNOWN_PLATFORM_BRANCHES`);

console.log(`${files.length} files checked; ${seen.size} split files still branch on ios (baseline ${KNOWN_PLATFORM_BRANCHES.size})`);
for (const f of fails) console.log('FAIL ' + f);
console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
