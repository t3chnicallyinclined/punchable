// Static test for the app's shape under "one notebook" (docs/design/SPEC-2026-11-17-one-notebook.md §2, §3; replaces
// tests/tabsShell.test.mjs, retired with the tab bar), read from the files: no tab bar anywhere; one stack with
// Notebook at its root; every old tab root pops to Notebook with its filter; the avatar on Notebook, Settings and You
// in one stack behind it; the marks tray gone and its URL kept as a redirect; the page's bar on the page and the
// draft a sheet; the input bar never left to a tab-bar accessory that no longer exists; a set's Record and Film open
// the walk-through. A layout check, not a render.
//   node --experimental-strip-types --no-warnings tests/notebookShell.test.mjs          the suite (the working tree)
//   node --experimental-strip-types --no-warnings tests/notebookShell.test.mjs --red    the red arm: the same checks
//                                                                                       over main before this
//                                                                                       branch (6088fc1, Bits ·
//                                                                                       Sets · Tapes) must FAIL
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const red = process.argv.includes("--red");
const BEFORE = "6088fc1"; // main before one notebook: the three tabs
const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const NL = String.fromCharCode(10);
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const git = (args) => execFileSync("git", args, { cwd: app, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 64 * 1024 * 1024 });
function src(p) {
  if (!red) return existsSync(join(app, p)) ? readFileSync(join(app, p), "utf8") : "";
  try { return git(["show", `${BEFORE}:app/${p}`]); } catch { return ""; }
}
const isSource = (f) => f.endsWith(".ts") || f.endsWith(".tsx");
/** Every source file's path under app/src, with forward slashes (the tree before the branch: from git). */
function names() {
  if (red) return git(["ls-tree", "-r", "--name-only", BEFORE, "--", "src"]).split(NL).filter(isSource);
  const walk = (d) => readdirSync(d).flatMap((n) => { const p = join(d, n); return statSync(p).isDirectory() ? walk(p) : [p]; });
  return walk(join(app, "src")).filter(isSource).map((f) => f.split("\\").join("/"));
}
const files = names();
/** The files that draw a tab bar. One `git grep` over the old tree: a `git show` per file takes minutes on Windows. */
function tabFiles() {
  if (!red) return files.filter((f) => { const t = readFileSync(f, "utf8"); return t.includes("expo-router/native-tabs") || t.includes("<NativeTabs"); });
  try { return git(["grep", "-l", "-E", "expo-router/native-tabs|<NativeTabs", BEFORE, "--", "src"]).split(NL).filter(Boolean); } catch { return []; }
}

// ---- no tab bar anywhere
const tabby = tabFiles();
check(tabby.length === 0, `nothing draws a tab bar (got ${tabby.length ? tabby.join(", ") : "none"})`);
check(!files.some((f) => f.includes("/(tabs)/")), "the (tabs) group is gone");
const web = src("src/lib/Screen.tsx");
check(!/bottom-nav|BottomNav|accessibilityRole="tab"/.test(web), "the web shell draws no tabs");
// ---- one stack, Notebook at its root
const layout = src("src/app/(nb)/_layout.tsx");
check(layout.includes("anchor: 'index'") && layout.includes("<Stack "), "one stack, anchored on Notebook");
check(src("src/app/_layout.tsx").includes('name="(nb)"'), "the root layout mounts the one stack");
const home = src("src/app/(nb)/index.tsx");
check(home.includes('title="Notebook"') && home.includes("N.notebook(") && home.includes("N.FILTERS"), "/ is Notebook: the one list with its filter");
check(home.includes("<Avatar "), "the avatar is on Notebook");
check(home.includes('testID="nb-talk"') && home.includes('testID="nb-new"'), "the microphone and the pencil are at the bottom of Notebook");
// ---- every old tab root pops to Notebook with its filter
for (const k of ["bits", "sets", "tapes", "filmed"]) {
  const t = src(`src/app/(nb)/${k}.tsx`);
  check(t.includes(`<PopTo href={OLD_ROOTS['/${k}']} />`) && !t.includes("<Redirect"), `/${k} pops to Notebook with its filter (never a Redirect)`);
}
// ---- Settings and You behind the avatar
const me = src("src/app/(me)/_layout.tsx");
check(src("src/app/_layout.tsx").includes('name="(me)"') && me.includes('name="settings"') && me.includes('name="profile"'), "Settings and You are one root stack behind the avatar");
// ---- the page: its bar, its marks under it, the tray gone
const page = src("src/app/(nb)/joke/[id].tsx");
check(page.includes("<PageBar ") && page.includes("<MarksList ") && page.includes("<PageTitle "), "a bit is a page: its title field, its bar, its marks listed under it");
check(!files.some((f) => f.endsWith("/MarkTray.tsx") || f.endsWith("/DocMarkBar.tsx") || f.endsWith("/MarkBar.tsx")), "the marks tray and the old marks bar are gone");
check(src("src/app/marks.tsx").includes("<Redirect "), "/marks is a redirect to the page it names");
const draftLine = layout.split(NL).find((l) => l.includes('name="joke/[id]/draft"')) || "";
check(draftLine.includes("presentation: 'modal'"), "the new draft is a sheet over the page");
const draft = src("src/app/(nb)/joke/[id]/draft.tsx");
check(["Keep mine", "Use it", "Put mine back", "Try again"].every((w) => draft.includes(w)), "the draft: Keep mine, Use it, Put mine back, Try again");
// ---- the conversation's input bar is always drawn by its screen (SPEC 2026-11-16 §5 R11: with the tab bar's
// accessory on and nothing drawing it, no bar was drawn on iOS 26)
const acc = src("src/lib/accessory.native.ts").split(NL).filter((l) => !l.trimStart().startsWith("//")).join(NL);
check(acc.includes("export const ACCESSORY = false;") && !acc.includes("isLiquidGlassAvailable"), "the input bar is never left to a tab-bar accessory");
// ---- what the old first tab did at launch still happens
check(src("src/app/_layout.tsx").includes("flushScriptDrafts()"), "a rant script kept on the phone is sent from the root layout");
check(home.includes("useTonight()"), "Notebook keeps the inbox count right from launch");
// ---- a set's Record and Film open the walk-through
const list = src("src/app/(nb)/list/[id].tsx");
check(list.includes("pathname: '/run/[id]'") && list.includes("mode: 'sound'") && list.includes("mode: 'video'"), "a set's Record and Film open /run/<id>");

if (red) {
  const caught = fails.length > 0;
  console.log(`${caught ? "PASS" : "FAIL"} red arm: the tree before the branch fails ${fails.length} check(s)`);
  process.exit(caught ? 0 : 1);
}
console.log(fails.length ? `${NL}${fails.length} FAILED` : `${NL}all passed`);
process.exit(fails.length ? 1 : 0);
