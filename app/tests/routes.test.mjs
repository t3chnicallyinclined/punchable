// Route resolution after the move into route groups (docs/design/HIG.md §2.1, B2): every URL in REQUIRED_URLS
// resolves to exactly one route file on disk, no two files claim one URL, and the static pages exist in app/public.
//   node --experimental-strip-types --no-warnings tests/routes.test.mjs
import { readdirSync, statSync, existsSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { REQUIRED_URLS, STATIC_URLS, duplicates, matches, patternOf, resolve } from "../src/rules/routeTable.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const here = fileURLToPath(new URL(".", import.meta.url));
const APP = join(here, "..", "src", "app");

const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
const files = walk(APP).map((p) => relative(APP, p).replace(/\\/g, "/"));
const table = files.map((f) => [f, patternOf(f)]).filter(([, p]) => p);
const patterns = table.map(([, p]) => p);

// the rule itself
check(patternOf("(tabs)/(bits)/joke/[id].tsx") === "/joke/:id", "a group vanishes, [id] is a param");
check(patternOf("(tabs)/(tonight)/index.tsx") === "/", "the tonight index is the root");
check(patternOf("(tabs)/(bits)/map/index.tsx") === "/map", "index is its directory");
check(patternOf("(tabs)/_layout.tsx") === null && patternOf("+not-found.tsx") === null && patternOf("x/readme.md") === null, "layouts, specials and non-code are not pages");
check(matches("/joke/joke%3Aairport-bit?x=1", "/joke/:id") && !matches("/joke", "/joke/:id") && !matches("/joke/a/b", "/joke/:id"), "a param takes exactly one segment");
check(matches("/a/b/c", "/a/*rest") && !matches("/x/b", "/a/*rest"), "a catch-all takes the remainder");

// the tree on disk
check(table.length >= 38, `${table.length} route files found under src/app`);
check(Object.keys(duplicates(files)).length === 0, `no two files claim one URL ${JSON.stringify(duplicates(files))}`);
for (const url of REQUIRED_URLS) {
  const hits = resolve(url, patterns);
  check(hits.length === 1, `${url} -> ${hits.length === 1 ? table.find(([, p]) => p === hits[0])[0] : `${hits.length} routes ${JSON.stringify(hits)}`}`);
}
for (const [url, file] of Object.entries(STATIC_URLS)) check(existsSync(join(here, "..", "public", file)), `${url} -> public/${file} (static, outside the router)`);

// the tab groups hold the three roots and the root stack holds the full-screen tasks
const fileOf = (url) => table.find(([, p]) => matches(url, p))?.[0] || "";
check(fileOf("/").startsWith("(tabs)/(tonight)/") && fileOf("/bits").startsWith("(tabs)/(bits)/") && fileOf("/sets").startsWith("(tabs)/(sets)/"), "the three tab roots live in their groups");
check(fileOf("/joke/x").startsWith("(tabs)/(bits)/") && fileOf("/list/x").startsWith("(tabs)/(sets)/") && fileOf("/inbox").startsWith("(tabs)/(tonight)/"), "pushed screens live under their tab (Back goes to it)");
check(!fileOf("/stage/x").startsWith("(tabs)") && !fileOf("/record").startsWith("(tabs)") && !fileOf("/review").startsWith("(tabs)"), "the stage, the recorder and the review are root-level full-screen modals");
check(!fileOf("/create").startsWith("(tabs)") && !fileOf("/add/x").startsWith("(tabs)"), "the form sheets (create, add a joke) are root-level routes that cover the tab bar");
check(!fileOf("/login").startsWith("(tabs)") && !fileOf("/auth/callback").startsWith("(tabs)") && !fileOf("/auth/logout").startsWith("(tabs)"), "login and the auth routes stay at the root, outside the tabs");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
