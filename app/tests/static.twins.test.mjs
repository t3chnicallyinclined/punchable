// Static check: every platform-split module has a file for every platform (docs/ENGINEERING.md "Platform splits").
// Metro picks `X.ios` > `X.native` > `X` on the phone and `X.web` > `X` on the web. A `.native.tsx` with no plain or
// `.web` twin is a web build that fails to resolve; a `.web.tsx` with no plain or `.native` twin is a phone that does.
// Also: a split file never imports its own basename (tests/selfImport.test.mjs covers the `.native` case; this one
// covers `.ios`, `.android` and `.web` too), a split pair never has BOTH `.native` and `.ios` without `.android`
// (the Android resolution would silently take `.native` while iOS takes `.ios`: two different components, one name),
// and a split pair shares ONE extension: Metro loops extensions outside platforms, `.ts` before `.tsx`, so with
// `X.native.tsx` beside `X.ts` the phone loads `X.ts` (the web file) and never the `.native` one (ENGINEERING.md §1).
//   node --experimental-strip-types --no-warnings tests/static.twins.test.mjs
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = resolve(fileURLToPath(new URL('../src', import.meta.url)));
const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const rel = (p) => relative(SRC, p).replace(/\\/g, '/');
const fails = [];
const check = (c, m) => { if (!c) fails.push(m); };

const files = walk(SRC).filter((p) => /\.(ts|tsx)$/.test(p) && !/\.d\.ts$/.test(p));
const split = files.map((p) => ({ p, m: basename(p).match(/^(.*)\.(ios|android|native|web)\.(tsx?)$/) })).filter((x) => x.m);
const exists = (dir, base, variant, ext) => ['tsx', 'ts'].some((e) => existsSync(join(dir, `${base}${variant ? '.' + variant : ''}.${e}`)) && (ext || true));

for (const { p, m } of split) {
  const [, base, variant] = m;
  const dir = dirname(p);
  const plain = exists(dir, base, '');
  if (variant === 'native' || variant === 'ios' || variant === 'android') {
    check(plain || exists(dir, base, 'web'), `${rel(p)} has no web twin: add ${base}.tsx (or ${base}.web.tsx)`);
  }
  if (variant === 'web') {
    check(plain || exists(dir, base, 'native') || (exists(dir, base, 'ios') && exists(dir, base, 'android')), `${rel(p)} has no phone twin: add ${base}.tsx or ${base}.native.tsx`);
  }
  if (variant === 'ios') {
    check(!exists(dir, base, 'native') || exists(dir, base, 'android'), `${rel(p)}: with ${base}.native.* present, Android gets .native and iOS gets .ios; add ${base}.android.* or drop one`);
  }
  // a split pair shares one extension: X.native.tsx beside X.ts means iOS resolves X.ts (.ts comes before .native.tsx)
  const ext = m[3];
  const other = ext === 'tsx' ? 'ts' : 'tsx';
  for (const v of ['', 'native', 'web', 'ios', 'android']) {
    if (v === variant) continue;
    const twin = join(dir, `${base}${v ? '.' + v : ''}.${other}`);
    check(!existsSync(twin), `${rel(p)} and ${rel(twin)} differ in extension: Metro tries every .${other} before any .${ext}, so one platform loads the wrong twin; give both .${ext} or both .${other}`);
  }
  // never import your own basename from a split file (on some platform that file is you)
  const own = new RegExp(`from\\s+['"]\\./${base.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\$&')}['"]`);
  check(!own.test(readFileSync(p, 'utf8')), `${rel(p)} imports './${base}' (itself on the platform that resolves to it); import the Base/Svg module directly`);
}

console.log(`${split.length} platform-split files checked`);
for (const f of fails) console.log('FAIL ' + f);
console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
