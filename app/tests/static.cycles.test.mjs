// Static check: no import cycles between modules under src/ (docs/ENGINEERING.md "Import cycles").
// A cycle is how a module ends up reading another's export before it is initialised: on Hermes that is a silent
// `undefined` (a component that renders nothing, a function that is not a function) or Metro's "Require cycle" warning,
// and it is the next of kin of the 2026-10-28 self-import crash (Metro resolved './Icon' to Icon.native.tsx itself).
//
// The graph is resolved the way Metro resolves it, once per platform (ios: .ios > .native > plain; web: .web > plain),
// over value imports only (`import type` / `export type` vanish at build time). Strongly connected components with more
// than one file, or a self-edge, are cycles. KNOWN_CYCLES is the baseline: a cycle not in it fails; a baseline entry
// that no longer exists also fails (delete the entry, so the list stays true). The goal is an empty baseline.
//   node --experimental-strip-types --no-warnings tests/static.cycles.test.mjs [--list]
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = resolve(fileURLToPath(new URL('../src', import.meta.url)));
const LIST = process.argv.includes('--list');

/** Cycles that exist today (each entry: the member files, sorted). Shrink this; never grow it. */
const KNOWN_CYCLES = { ios: [], web: [] }; // 2026-10-28: none. Keep it that way.

const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const files = walk(SRC).filter((p) => /\.(ts|tsx)$/.test(p) && !/\.d\.ts$/.test(p));
const rel = (p) => relative(SRC, p).replace(/\\/g, '/');

/** Module specifiers a file imports at runtime (not `import type`, not `export type`). */
function specifiers(src) {
  const out = [];
  const code = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  const rx = /(?:^|\n)\s*(import|export)\s+(type\s+)?(?:[\s\S]*?\s+from\s+)?['"]([^'"]+)['"]/g;
  let m;
  while ((m = rx.exec(code))) if (!m[2]) out.push(m[3]);
  const dyn = /(?:import|require)\(\s*['"]([^'"]+)['"]\s*\)/g;
  while ((m = dyn.exec(code))) out.push(m[1]);
  return out;
}

const PLATFORM_EXT = { ios: ['.ios', '.native', ''], web: ['.web', ''] };
function resolveSpec(fromFile, spec, platform) {
  let base;
  if (spec.startsWith('@/')) base = join(SRC, spec.slice(2));
  else if (spec.startsWith('.')) base = resolve(dirname(fromFile), spec);
  else return null; // a package: not part of this graph
  for (const plat of PLATFORM_EXT[platform]) {
    for (const ext of ['.tsx', '.ts']) {
      const f = base + plat + ext;
      if (existsSync(f)) return f;
    }
  }
  for (const plat of PLATFORM_EXT[platform]) {
    for (const ext of ['.tsx', '.ts']) {
      const f = join(base, 'index' + plat + ext);
      if (existsSync(f)) return f;
    }
  }
  return null;
}

/** The files Metro would ever load on this platform: a plain file is shadowed by its .native/.ios/.web twin. */
function visibleOn(platform) {
  const set = new Set();
  for (const f of files) {
    const m = f.match(/^(.*?)(\.(ios|android|native|web))?\.(tsx?)$/);
    if (!m) continue;
    const variant = m[3] || '';
    if (platform === 'ios' && (variant === 'web' || variant === 'android')) continue;
    if (platform === 'web' && (variant === 'ios' || variant === 'native' || variant === 'android')) continue;
    if (variant === '') {
      // shadowed if a platform twin exists
      const twins = PLATFORM_EXT[platform].filter(Boolean).map((v) => m[1] + v + '.' + m[4]);
      if (twins.some((t) => existsSync(t))) continue;
    }
    set.add(f);
  }
  return set;
}

function graph(platform) {
  const nodes = visibleOn(platform);
  const edges = new Map();
  for (const f of nodes) {
    const to = new Set();
    for (const s of specifiers(readFileSync(f, 'utf8'))) {
      const r = resolveSpec(f, s, platform);
      if (r && nodes.has(r)) to.add(r);
    }
    edges.set(f, to);
  }
  return edges;
}

/** Tarjan: strongly connected components; a cycle is an SCC of size > 1 or a node with a self-edge. */
function cycles(edges) {
  let index = 0;
  const idx = new Map(), low = new Map(), on = new Set(), stack = [], out = [];
  const strong = (v) => {
    idx.set(v, index); low.set(v, index); index++; stack.push(v); on.add(v);
    for (const w of edges.get(v) || []) {
      if (!idx.has(w)) { strong(w); low.set(v, Math.min(low.get(v), low.get(w))); }
      else if (on.has(w)) low.set(v, Math.min(low.get(v), idx.get(w)));
    }
    if (low.get(v) === idx.get(v)) {
      const comp = [];
      let w;
      do { w = stack.pop(); on.delete(w); comp.push(w); } while (w !== v);
      if (comp.length > 1 || (edges.get(v) || new Set()).has(v)) out.push(comp.map(rel).sort());
    }
  };
  for (const v of edges.keys()) if (!idx.has(v)) strong(v);
  return out.sort((a, b) => a.join().localeCompare(b.join()));
}

// --self-test: the detector must catch a planted cycle (a -> b -> a) and a self-edge, else it proves nothing
if (process.argv.includes('--self-test')) {
  const a = '/x/a.tsx', b = '/x/b.tsx', c = '/x/c.tsx';
  const got = cycles(new Map([[a, new Set([b])], [b, new Set([a])], [c, new Set([c])]]));
  const ok = got.length === 2;
  console.log(`${ok ? 'PASS' : 'FAIL'} self-test: planted a<->b and c->c, found ${got.length} cycle(s)`);
  const specs = specifiers(["import type { A } from './a';", "import { B } from './b';", "export * from './c';", "export type { D } from './d';", "const e = require('./e');", "const f = await import('./f');"].join('\n'));
  const ok2 = specs.join() === './b,./c,./e,./f';
  console.log(`${ok2 ? 'PASS' : 'FAIL'} self-test: value imports only -> ${specs.join(' ')}`);
  process.exit(ok && ok2 ? 0 : 1);
}

const fails = [];
let total = 0;
for (const platform of ['ios', 'web']) {
  const edges = graph(platform);
  const found = cycles(edges);
  total += found.length;
  const key = (c) => c.join(' -> ');
  const known = new Set((KNOWN_CYCLES[platform] || []).map(key));
  const seen = new Set(found.map(key));
  if (LIST) for (const c of found) console.log(`${platform}: ${key(c)}`);
  for (const c of found) if (!known.has(key(c))) fails.push(`NEW cycle on ${platform}: ${key(c)}`);
  for (const k of known) if (!seen.has(k)) fails.push(`stale baseline on ${platform} (fixed? delete it from KNOWN_CYCLES): ${k}`);
  console.log(`${fails.length ? 'FAIL' : 'PASS'} ${platform}: ${edges.size} modules, ${found.length} cycle(s), ${known.size} in the baseline`);
}
for (const f of fails) console.log('FAIL ' + f);
console.log(fails.length ? `${fails.length} FAILED` : `ALL PASS (${total} known cycle(s) remain; shrink KNOWN_CYCLES)`);
process.exit(fails.length ? 1 : 0);
