// Embed texts with Xenova/bge-small-en-v1.5 (MIT), CLS pooling, L2-normalised, 384 dims.
// Candidates only, never a verdict. Never use jina models here.
// Reuses the transformers.js install in spikes/01-vector-graph (read-only, no new deps).
// Usage: node embed.mjs <in.json [{id,text}]> <out.f32>
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL, fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SPIKE = process.env.INFER_SPIKE || path.resolve(HERE, '..', 'spikes', '01-vector-graph');
const req = createRequire(path.join(SPIKE, 'package.json'));
const { pipeline, env } = await import(pathToFileURL(req.resolve('@huggingface/transformers')).href);
env.allowRemoteModels = true;
const pipe = await pipeline('feature-extraction', 'Xenova/bge-small-en-v1.5', { dtype: 'fp32' });
const items = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const buf = new Float32Array(items.length * 384);
for (let i = 0; i < items.length; i += 16) {
  const chunk = items.slice(i, i + 16).map(t => t.text || ' ');
  const rows = (await pipe(chunk, { pooling: 'cls', normalize: true })).tolist();
  rows.forEach((r, j) => buf.set(r, (i + j) * 384));
}
fs.writeFileSync(process.argv[3], Buffer.from(buf.buffer));
console.log(`embedded ${items.length}`);
