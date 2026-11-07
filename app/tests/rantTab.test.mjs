// Static test for the Rant tab and the Tonight-parts move (SPEC 2026-11-06 R1, R2, R4; slice 1), read from the files:
// the tab bar is Rant · Bits · Sets with megaphone.fill first and the "N to sort" badge on Bits; Tonight's set list bar
// and Record a set open Sets above its segments; "N to sort" and Record a memo are on Bits, the inbox in Bits' stack;
// the Rant screen is his creative space (§12): where he left off, Topics as pages, the script a push, Material and Claude
// on a line as sheets. A layout check, not a render.
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

// ---- the Rant screen (§12.2: the creative space): where he left off, Topics, Filmed; no red disc, no Drafts form
const rantScreen = src("src/app/(tabs)/(rant)/index.tsx");
check(rantScreen.includes('title="Rant"') && rantScreen.includes('testID="rant-leftoff"') && rantScreen.includes("Keep writing"), "the Rant screen opens on where he left off, in his words, with Keep writing");
check(rantScreen.includes('title="Topics"') && rantScreen.includes("openTopic(t)") && rantScreen.includes("'/new-topic'"), "Topics push the topic page; + is New topic");
check(rantScreen.includes("useTonight()"), "the badge is fed from launch");
check(!/Ask your material|tonight-set|Record a set/.test(rantScreen), "none of Tonight's parts stayed on the first tab");
check(!rantScreen.includes("'/rant/new'") && !/Film a rant|useTopicCounts|title="Drafts"|title="Finished"/.test(rantScreen), "the red disc row, the 12-bits count and the Drafts and Finished sections are gone (§12.8)");

// ---- the topic is a page, the script a push, a bit opened from a topic stays in Rant (§12.7)
check(has("src/app/(tabs)/(rant)/topic/[topic]/index.tsx") && has("src/app/(tabs)/(rant)/topic/[topic]/script.tsx") && has("src/app/(tabs)/(rant)/new-topic.tsx"), "the topic, its script and New topic are pushes in the Rant stack");
check(/export \{ default \} from '.*\(bits\)\/joke\/\[id\]'/.test(src("src/app/(tabs)/(rant)/topic/[topic]/bit/[id].tsx")), "a bit opened from a topic is the bit page, in the Rant stack");
const topicPage = src("src/app/(tabs)/(rant)/topic/[topic]/index.tsx");
check(["Find a bit to add", "Jot a thought", "Kept · ", "Found · ", "Dropped · ", "Find more"].every((w) => topicPage.includes(w)), "the topic page: search, jot, Kept, Found, Dropped, Find more");
const root = src("src/app/_layout.tsx");
check(root.includes('name="material/[topic]"') && root.includes('name="rant-line"') && !root.includes('name="rant/new"'), "Material and Claude on a line are sheets; Write a rant's sheet is gone");

if (fails.length) {
  console.log(`\n${fails.length} FAILED${red ? " (the red arm: the tree before the move must fail)" : ""}`);
  process.exit(red ? 0 : 1);
}
if (red) { console.log("\nthe red arm PASSED over the old tree: the suite cannot fail"); process.exit(1); }
console.log("\nall passed");
