// Embedding sidecar for Punchable's SurrealDB layer: bge-small-en-v1.5 (MIT), CLS pooling, L2-normalised, 384-d,
// the same model and settings as tools/kb/kb_embed.mjs, so query vectors match the stored unit vectors.
// Reuses the transformers.js install (and its cached model) in spikes/01-vector-graph. Remote downloads are OFF:
// nothing is fetched and no text leaves the machine.
//
// Protocol: one JSON object per line on stdin {id, texts:[...], query:bool} -> one line on stdout {id, vecs:[[...]]}.
// Queries get BGE's retrieval instruction; stored passages never do.
import path from 'node:path';
import readline from 'node:readline';
import { createRequire } from 'node:module';
import { pathToFileURL, fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SPIKE = path.resolve(HERE, '..', 'spikes', '01-vector-graph');
const MODEL = 'Xenova/bge-small-en-v1.5';
const QUERY_PREFIX = 'Represent this sentence for searching relevant passages: ';

const req = createRequire(path.join(SPIKE, 'package.json'));
const { pipeline, env } = await import(pathToFileURL(req.resolve('@huggingface/transformers')).href);
env.allowRemoteModels = false;
const pipe = await pipeline('feature-extraction', MODEL, { dtype: 'fp32' });
process.stdout.write(JSON.stringify({ ready: true, model: MODEL }) + '\n');

const rl = readline.createInterface({ input: process.stdin });
for await (const line of rl) {
  if (!line.trim()) continue;
  let msg;
  try {
    msg = JSON.parse(line);
    const texts = (msg.texts || []).map((t) => (msg.query ? QUERY_PREFIX : '') + (t || ' '));
    const vecs = [];
    for (let i = 0; i < texts.length; i += 16) {
      const out = await pipe(texts.slice(i, i + 16), { pooling: 'cls', normalize: true });
      vecs.push(...out.tolist());
    }
    process.stdout.write(JSON.stringify({ id: msg.id, vecs }) + '\n');
  } catch (e) {
    process.stdout.write(JSON.stringify({ id: msg && msg.id, error: String(e && e.message || e) }) + '\n');
  }
}
