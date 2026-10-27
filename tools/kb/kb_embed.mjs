// Step 2b: embed cache/texts.json with Xenova/bge-small-en-v1.5 (MIT), CLS pooling,
// L2-normalised, 384 dims. Used for CANDIDATE generation only, never as a verdict.
// Never use jina models here.
// Reuses the transformers.js install from spikes/01-vector-graph (no new deps).
// Writes cache/vectors.f32 (row-major float32) + cache/vectors_ids.json.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL, fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SPIKE = path.resolve(HERE, '..', '..', 'spikes', '01-vector-graph');
const MODEL = 'Xenova/bge-small-en-v1.5';

const req = createRequire(path.join(SPIKE, 'package.json'));
const entry = req.resolve('@huggingface/transformers');
const { pipeline, env } = await import(pathToFileURL(entry).href);
env.allowRemoteModels = true;
const pipe = await pipeline('feature-extraction', MODEL, { dtype: 'fp32' });

// optional args: <texts.json> <out prefix> (default cache/texts.json -> cache/vectors.*)
const IN = process.argv[2] || path.join(HERE, 'cache', 'texts.json');
const OUT = process.argv[3] || path.join(HERE, 'cache', 'vectors');
const texts = JSON.parse(fs.readFileSync(IN, 'utf8'));
const dim = 384;
const buf = new Float32Array(texts.length * dim);
const t0 = Date.now();
const B = 16;
for (let i = 0; i < texts.length; i += B) {
  const chunk = texts.slice(i, i + B).map(t => t.text || ' ');
  const out = await pipe(chunk, { pooling: 'cls', normalize: true });
  const rows = out.tolist();
  rows.forEach((r, j) => buf.set(r, (i + j) * dim));
  if ((i / B) % 25 === 0) process.stdout.write(`${i}/${texts.length} ${((Date.now() - t0) / 1000).toFixed(0)}s\n`);
}
fs.writeFileSync(OUT + '.f32', Buffer.from(buf.buffer));
fs.writeFileSync(OUT + '_ids.json', JSON.stringify(texts.map(t => t.id)));
console.log(`done ${texts.length} in ${((Date.now() - t0) / 1000).toFixed(1)}s, model ${MODEL}`);
