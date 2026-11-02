// Static check: every worklet in src/rules, compiled the way Metro compiles it for the phone (babel-preset-expo + the
// react-native-worklets plugin), loads with a complete closure. The plugin turns each 'worklet' function into a factory
// that runs when the module loads and captures the names it uses AT THAT MOMENT: a worklet that calls a function or
// constant declared further down the file captures `undefined`, and the first call reaching it throws "undefined is not
// a function" on the UI thread (the cloud's resolveTap did, iOS dev review 2026-11-02). The unit tests import the
// TypeScript without the plugin, so only this check can see it. Rule: a worklet names only what is declared above it.
//   node --experimental-strip-types --no-warnings tests/static.worklets.test.mjs [--self-test]
import { createRequire } from 'node:module';
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const APP = resolve(fileURLToPath(new URL('..', import.meta.url)));
const RULES = join(APP, 'src', 'rules');
const req = createRequire(join(APP, 'package.json'));
const babel = req('@babel/core');

function compile(src, filename) {
  return babel.transformSync(src, {
    filename, cwd: APP, root: APP, babelrc: false, configFile: false,
    presets: [req.resolve('babel-preset-expo')],
    plugins: [req.resolve('react-native-worklets/plugin')],
    caller: { name: 'metro', platform: 'ios', bundler: 'metro' },
  }).code;
}

/** Load compiled CommonJS and list, per exported worklet, the closure names that are undefined. */
function holes(code) {
  const module = { exports: {} };
  new Function('require', 'exports', 'module', code)(req, module.exports, module);
  const out = [];
  let worklets = 0;
  for (const [name, v] of Object.entries(module.exports)) {
    if (typeof v !== 'function' || !('__workletHash' in v || v.__closure)) continue;
    worklets++;
    const missing = Object.entries(v.__closure || {}).filter(([, c]) => c === undefined).map(([k]) => k);
    if (missing.length) out.push(`${name} captures undefined: ${missing.join(', ')}`);
  }
  return { worklets, out };
}

if (process.argv.includes('--self-test')) {
  const planted = "export function early(a: number): number {\n  'worklet';\n  return later(a);\n}\nexport function later(a: number): number {\n  'worklet';\n  return a;\n}\n";
  const ok = "export function first(a: number): number {\n  'worklet';\n  return a;\n}\nexport function second(a: number): number {\n  'worklet';\n  return first(a);\n}\n";
  // the plugin reads the source file from disk, so the planted modules are written to a temp dir first
  const dir = mkdtempSync(join(tmpdir(), 'pk-worklets-'));
  const at = (name, text) => { const f = join(dir, name); writeFileSync(f, text); return compile(text, f); };
  const bad = holes(at('planted.ts', planted));
  const good = holes(at('ok.ts', ok));
  rmSync(dir, { recursive: true, force: true });
  const pass = bad.out.length === 1 && /early/.test(bad.out[0]) && good.out.length === 0 && good.worklets === 2;
  console.log(`${pass ? "PASS" : "FAIL"} self-test: a worklet naming a later function is caught (${bad.out.join("; ") || "not caught"}); the right order passes (${good.worklets} worklets, ${good.out.join("; ") || "no holes"})`);
  process.exit(pass ? 0 : 1);
}

const fails = [];
let checked = 0;
for (const f of readdirSync(RULES).filter((n) => n.endsWith('.ts') && !n.endsWith('.d.ts'))) {
  const p = join(RULES, f);
  const src = readFileSync(p, 'utf8');
  if (!/['"]worklet['"]/.test(src)) continue;
  const { worklets, out } = holes(compile(src, p));
  checked += worklets;
  if (!worklets) fails.push(`${f}: has 'worklet' directives but no exported worklet after compiling (is the plugin running?)`);
  for (const o of out) fails.push(`${f}: ${o}`);
}
if (fails.length) { for (const f of fails) console.log(`FAIL ${f}`); process.exit(1); }
console.log(`ALL PASS (${checked} worklets in src/rules load with complete closures)`);
