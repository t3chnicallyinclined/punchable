// Static test for the Rant tab and the Tonight-parts move (SPEC 2026-11-06 R1, R2, R4; slice 1), read from the files:
// the tab bar is Rant · Bits · Sets with megaphone.fill first and the "N to sort" badge on Bits; Tonight's set list bar
// and Record a set open Sets above its segments; "N to sort" and Record a memo are on Bits, the inbox in Bits' stack;
// the Rant screen has Film a rant, My topics, Drafts and Finished and opens Write a rant. A layout check, not a render.
//   node --experimental-strip-types --no-warnings tests/rantTab.test.mjs          the suite (the working tree)
//   node --experimental-strip-types --no-warnings tests/rantTab.test.mjs --red    the red arm: the same checks over the
//                                                                                tree before the move (0cf224b, Tonight
//                                                                                first) must FAIL
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const red = process.argv.includes("--red");
const BEFORE = "0cf224b"; // main before the Rant tab: Tonight was the first tab
const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

// the files as they are, or (--red) as they were: the (rant) group was the (tonight) group
const old = (p) => p.replace("(rant)", "(tonight)");
const git = (args) => execFileSync("git", args, { cwd: app, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
function src(p) {
  if (!red) return existsSync(join(app, p)) ? readFileSync(join(app, p), "utf8") : "";
  try { return git(["show", `${BEFORE}:app/${old(p)}`]); } catch { return ""; }
}
function has(p) {
  if (!red) return existsSync(join(app, p));
  try { git(["cat-file", "-e", `${BEFORE}:app/${p}`]); return true; } catch { return false; }
}

// ---- the tab bar (R1, R4)
const tabs = src("src/lib/TabsShell.native.tsx");
const order = [...tabs.matchAll(/NativeTabs\.Trigger name="\(([a-z]+)\)"/g)].map((m) => m[1]);
check(order.join(",") === "rant,bits,sets", `the tab bar is Rant · Bits · Sets (got ${order.join(" · ") || "none"})`);
check(/Label>Rant<[\s\S]*?sf="megaphone\.fill"/.test(tabs) && !/Label>Tonight</.test(tabs), "Rant is labelled Rant with megaphone.fill; no Tonight tab");
const bitsTrigger = tabs.split('name="(bits)"')[1]?.split("</NativeTabs.Trigger>")[0] || "";
check(/Trigger\.Badge/.test(bitsTrigger), "the \"N to sort\" badge is on the Bits tab");

// ---- the groups: URLs unchanged, the inbox under Bits (R2)
check(has("src/app/(tabs)/(rant)/index.tsx") && !has("src/app/(tabs)/(tonight)/index.tsx"), "the first tab's root is (rant)/index.tsx (the URL / is unchanged)");
check(has("src/app/(tabs)/(bits)/inbox.tsx") && !has("src/app/(tabs)/(rant)/inbox.tsx"), "the inbox lives in Bits' stack, where \"N to sort\" is");
check(["say.tsx", "import.tsx", "ask/index.tsx"].every((f) => has(`src/app/(tabs)/(bits)/${f}`)), "the conversation, Ask and Import sit in Bits' stack with the rows that open them (no tab switch)");
check(has("src/app/rant/new.tsx"), "Write a rant is the root-level sheet /rant/new");

// ---- Sets opens on tonight's list bar and Record a set, above the segments (R2)
const sets = src("src/app/(tabs)/(sets)/sets.tsx");
const segAt = sets.indexOf('testID="sets-seg"');
check(sets.includes("<TonightCard") && sets.indexOf("<TonightCard") < segAt, "Sets opens on tonight's set list bar, above Set lists · Recorded");
check(sets.includes('testID="sets-record-set"') && sets.indexOf('testID="sets-record-set"') < segAt, "Record a set is at the top of Sets");

// ---- Bits has "N to sort" and Record a memo in its ⋯ (R2)
const bits = src("src/app/(tabs)/(bits)/bits.tsx");
check(bits.includes('testID="to-sort"') && bits.includes("useTonight()"), "Bits shows \"N to sort\" (from the Tonight screen's request)");
const menu = bits.split('icon="ellipsis.circle"')[1]?.split("</Stack.Toolbar.Menu>\n      </Stack.Toolbar.Menu>")[0] || "";
check(/Record a memo/.test(menu) && /inline title="Sort"/.test(menu), "Record a memo is in Bits' ⋯, with the sort as its inline section");
check(bits.includes("placeholder: 'Search your bits, or ask'"), "Ask your material stays the Bits search");

// ---- the Rant screen (§2)
const rantScreen = src("src/app/(tabs)/(rant)/index.tsx");
check(rantScreen.includes('title="Rant"') && rantScreen.includes('testID="rant-film"'), "the Rant screen: a large title Rant and the Film a rant row");
check(["My topics", "Drafts", "Finished"].every((w) => rantScreen.includes(`title="${w}"`)), "My topics, Drafts and Finished");
check(rantScreen.includes("'/rant/new'") && rantScreen.includes("useTonight()"), "+ and a topic open Write a rant; the badge is fed from launch");
check(!/Ask your material|tonight-set|Record a set/.test(rantScreen), "none of Tonight's parts stayed on the first tab");

if (fails.length) {
  console.log(`\n${fails.length} FAILED${red ? " (the red arm: the tree before the move must fail)" : ""}`);
  process.exit(red ? 0 : 1);
}
if (red) { console.log("\nthe red arm PASSED over the old tree: the suite cannot fail"); process.exit(1); }
console.log("\nall passed");
