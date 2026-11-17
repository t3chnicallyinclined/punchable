// Static test for the tab bar Bits · Sets · Tapes (docs/design/SPEC-2026-11-16-bits-sets-tapes.md Slice 6; replaces
// tests/rantTab.test.mjs, retired with the Rant tab), read from the files: the tab bar's three tabs in order with the
// badge on Bits and Tapes' waveform; the web's roots the same; the avatar on every tab root; Settings and You in one
// stack behind it; the Rant scripts kept on the phone flushed from the root; a set's Record and Film open the
// walk-through (Slice 5). A layout check, not a render.
//   node --experimental-strip-types --no-warnings tests/tabsShell.test.mjs          the suite (the working tree)
//   node --experimental-strip-types --no-warnings tests/tabsShell.test.mjs --red    the red arm: the same checks over
//                                                                                  main before the slice (85614bb,
//                                                                                  Rant · Bits · Sets) must FAIL
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const red = process.argv.includes("--red");
const BEFORE = "85614bb"; // main before Slices 5 and 6: the Rant tab first
const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const git = (args) => execFileSync("git", args, { cwd: app, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
function src(p) {
  if (!red) return existsSync(join(app, p)) ? readFileSync(join(app, p), "utf8") : "";
  try { return git(["show", `${BEFORE}:app/${p}`]); } catch { return ""; }
}

// ---- the tab bar
const tabs = src("src/lib/TabsShell.native.tsx");
const order = [...tabs.matchAll(/NativeTabs\.Trigger name="\(([a-z]+)\)"/g)].map((m) => m[1]);
check(order.join(",") === "bits,sets,tapes", `the tab bar is Bits · Sets · Tapes (got ${order.join(" · ") || "none"})`);
check(/Label>Tapes<[\s\S]*?sf="waveform"/.test(tabs) && !/Label>Rant</.test(tabs), "Tapes is labelled Tapes with waveform; no Rant tab");
const bitsTrigger = tabs.split('name="(bits)"')[1]?.split("</NativeTabs.Trigger>")[0] || "";
check(/Trigger\.Badge/.test(bitsTrigger), "the \"N to sort\" badge stays on Bits");
// ---- the web's roots
const top = src("src/lib/TopBar.tsx");
const roots = [...(top.split("export const ROOTS")[1] || "").split("];")[0].matchAll(/key: '([a-z]+)'/g)].map((m) => m[1]);
check(roots.join(",") === "bits,sets,tapes", `the web's tabs are Bits · Sets · Tapes (got ${roots.join(" · ") || "none"})`);
// ---- the avatar on every tab root, Settings and You behind it
for (const f of ["src/app/(tabs)/(bits)/bits.tsx", "src/app/(tabs)/(sets)/sets.tsx", "src/app/(tabs)/(tapes)/tapes.tsx"]) check(/<Avatar\b/.test(src(f)), `the avatar is on ${f.split("/").slice(-1)[0]}`);
check(/name="\(me\)"/.test(src("src/app/_layout.tsx")) && /name="settings"/.test(src("src/app/(me)/_layout.tsx")) && /name="profile"/.test(src("src/app/(me)/_layout.tsx")), "Settings and You are one root stack behind the avatar");
// ---- what the Rant tab did at launch now happens at the root
check(/flushScriptDrafts\(\)/.test(src("src/app/_layout.tsx")) && /export async function flushScriptDrafts/.test(src("src/lib/scriptFlush.ts")), "a rant script kept on the phone is sent from the root layout");
check(/useTonight\(\)/.test(src("src/app/(tabs)/(bits)/bits.tsx")), "the first tab (Bits) keeps the badge right from launch");
// ---- a set's Record and Film open the walk-through (Slice 5)
const list = src("src/app/(tabs)/(sets)/list/[id].tsx");
check(/pathname: '\/run\/\[id\]', params: \{ id, mode: 'sound' \}/.test(list) && /mode: 'video'/.test(list), "a set's Record and Film open /run/<id>");

if (red) {
  const caught = fails.length > 0;
  console.log(`${caught ? "PASS" : "FAIL"} red arm: the tree before the slice fails ${fails.length} check(s)`);
  process.exit(caught ? 0 : 1);
}
console.log(fails.length ? `\n${fails.length} FAILED` : "\nall passed");
process.exit(fails.length ? 1 : 0);
