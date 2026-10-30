// Static check: src/rules is the bottom layer (docs/ARCHITECTURE.md §7.1, docs/ENGINEERING.md §1). Every module there
// is pure, runs under node, and is imported with its explicit `.ts` extension, so:
//  1. no platform twins: `X.native.ts` / `X.ios.ts` / `X.android.ts` / `X.web.ts` would never be picked, because an
//     explicit `./X.ts` resolves the exact file first (Metro tries the path as written before any platform suffix);
//  2. `.ts` only, never `.tsx`: a rule has no JSX;
//  3. no value import of a package (react, react-native, expo-*): `import type` is erased and allowed;
//  4. every relative import names its `.ts` extension and stays inside src/rules (a rule never reaches up into
//     src/lib or src/app).
//   node --experimental-strip-types --no-warnings tests/static.rules.test.mjs
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const RULES = resolve(fileURLToPath(new URL('../src/rules', import.meta.url)));
const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const rel = (p) => relative(RULES, p).replace(/\\/g, '/');
const fails = [];
const check = (c, m) => { if (!c) fails.push(m); };

const files = walk(RULES).filter((p) => !/\.d\.ts$/.test(p));
check(files.length > 0, 'src/rules is empty');

/** Import and re-export specifiers, with whether each is type-only. */
function specifiers(src) {
  const out = [];
  const code = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  const rx = /(?:^|\n)\s*(import|export)\s+(type\s+)?(?:[\s\S]*?\s+from\s+)?['"]([^'"]+)['"]/g;
  let m;
  while ((m = rx.exec(code))) out.push({ spec: m[3], typeOnly: !!m[2] });
  const dyn = /(?:import|require)\(\s*['"]([^'"]+)['"]\s*\)/g;
  while ((m = dyn.exec(code))) out.push({ spec: m[1], typeOnly: false });
  return out;
}

for (const p of files) {
  const r = rel(p);
  check(/\.ts$/.test(r), `${r}: only .ts under src/rules (no JSX, no .tsx)`);
  check(!/\.(ios|android|native|web)\.tsx?$/.test(r), `${r}: no platform twins under src/rules (an explicit .ts import never resolves them)`);
  for (const { spec, typeOnly } of specifiers(readFileSync(p, 'utf8'))) {
    if (spec.startsWith('.')) {
      check(/\.ts$/.test(spec), `${r}: import '${spec}' must name its .ts extension (node runs these files as they are)`);
      check(!/^\.\.\//.test(spec), `${r}: import '${spec}' leaves src/rules; the rules layer imports nothing above it`);
    } else if (spec.startsWith('@/')) {
      check(spec.startsWith('@/rules/'), `${r}: import '${spec}' reaches outside src/rules`);
      check(/\.ts$/.test(spec), `${r}: import '${spec}' must name its .ts extension`);
    } else {
      check(typeOnly, `${r}: value import of '${spec}'; a rule imports no package (React, React Native, Expo): types only`);
    }
  }
}

// --self-test: the checks must catch a planted offender, else they prove nothing
if (process.argv.includes('--self-test')) {
  const bad = specifiers(["import { View } from 'react-native';", "import type { X } from 'expo-audio';", "import { a } from './a';", "import { b } from './b.ts';", "import { c } from '@/lib/c';"].join('\n'));
  const ok = bad.length === 5 && !bad[0].typeOnly && bad[1].typeOnly;
  console.log(`${ok ? 'PASS' : 'FAIL'} self-test: specifiers -> ${bad.map((x) => `${x.spec}${x.typeOnly ? ' (type)' : ''}`).join(', ')}`);
  process.exit(ok ? 0 : 1);
}

console.log(`${files.length} rules modules checked`);
for (const f of fails) console.log('FAIL ' + f);
console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
