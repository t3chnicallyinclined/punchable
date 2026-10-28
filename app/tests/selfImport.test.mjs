// A *.native.tsx file must never import './<its own basename>': on the phone Metro resolves that to the .native file
// itself, which recurses until the stack overflows and takes every screen down (2026-10-28, TestFlight builds 4-6).
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, basename } from 'node:path';
let fails = 0, checked = 0;
function walk(d) { for (const n of readdirSync(d)) { const p = join(d, n); if (statSync(p).isDirectory()) walk(p); else if (/\.native\.tsx?$/.test(n)) check(p); } }
function check(p) {
  checked++;
  const base = basename(p).replace(/\.native\.tsx?$/, '');
  const rx = new RegExp(`from ['"]\./${base.replace('.', '\.')}['"]`);
  if (rx.test(readFileSync(p, 'utf8'))) { console.log(`FAIL ${p} imports './${base}' (itself on the phone)`); fails++; }
}
walk(new URL('../src', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
console.log(fails ? `FAILED: ${fails} self-import(s) in ${checked} native files` : `PASS no self-imports in ${checked} native files`);
process.exit(fails ? 1 : 0);
