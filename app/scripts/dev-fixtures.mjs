// DEV ONLY. Copies the creative layer's own demo outputs (python -m creative demo-all) into public/dev-fixtures/,
// which the Expo dev server serves and src/lib/devFixtures.ts reads while a contract route isn't live yet.
// public/dev-fixtures/ is gitignored: it is the comic's material and never ships (the Docker build uses `git archive`).
//   node scripts/dev-fixtures.mjs          (npm run dev-fixtures)
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, "..", "public", "dev-fixtures");
const CREATIVE = process.env.CREATIVE_DIR || (process.env.HOME || process.env.USERPROFILE || ".") + "/punchable-data/kb/creative";
const DEMO = join(CREATIVE, "demo");
if (process.argv.includes("--clean")) {
  rmSync(OUT, { recursive: true, force: true });
  console.log("removed", OUT);
  process.exit(0);
}
if (!existsSync(DEMO)) {
  console.error(`no ${DEMO}: run \`python -m creative demo-all\` first`);
  process.exit(1);
}
mkdirSync(OUT, { recursive: true });
const files = readdirSync(DEMO).filter((f) => f.endsWith(".json") || f.endsWith(".geojson"));
for (const f of files) copyFileSync(join(DEMO, f), join(OUT, f.endsWith(".geojson") ? f.replace(/\.geojson$/, ".geojson.json") : f));
if (existsSync(join(CREATIVE, "storyboards.json"))) copyFileSync(join(CREATIVE, "storyboards.json"), join(OUT, "storyboards.json"));
console.log(`copied ${files.length + 1} fixture files to ${OUT}`);
