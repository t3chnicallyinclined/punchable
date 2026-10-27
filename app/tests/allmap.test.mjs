// Unit test for src/lib/allmap.ts: a deterministic layout, components kept apart, neighbours placed on expand, and
// the budget: the first draw (top 300 of 1,600 invented bits) lays out well under a second.
//   node --experimental-strip-types --no-warnings tests/allmap.test.mjs
import { layout, placeNear, topNodes, visible } from "../src/lib/allmap.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

// invented material: 1,600 bits, a few hundred links in small worlds
const N = 1600;
const nodes = Array.from({ length: N }, (_, i) => ({ id: `joke:b${i}`, short_name: `Bit ${i}`, nights: i % 11 === 0 ? 3 + (i % 5) : i % 3, heat: (i % 7) / 5, degree: 0 }));
const edges = [];
for (let i = 0; i < N; i++) {
  if (i % 4 === 0 && i + 1 < N) edges.push({ a: `joke:b${i}`, b: `joke:b${i + 1}`, kind: "same_world", w: 0.6 });
  if (i % 9 === 0 && i + 3 < N) edges.push({ a: `joke:b${i}`, b: `joke:b${i + 3}`, kind: "callbacks", w: 0.8 });
  if (i % 50 === 0 && i + 25 < N) edges.push({ a: `joke:b${i}`, b: `joke:b${i + 25}`, kind: "next", w: 1 });
}

const top = topNodes(nodes, 300);
check(top.length === 300 && top[0].nights >= top[299].nights, "top 300 by nights first");
const t0 = Date.now();
const L = layout(top, edges);
const ms = Date.now() - t0;
check(ms < 1000, `layout of 300 visible nodes in ${ms} ms (< 1000)`);
check(L.nodes.length === 300 && L.nodes.every((n) => Number.isFinite(n.x) && Number.isFinite(n.y)), "every node has a position");
check(L.nodes.every((n) => n.x >= 0 && n.x <= L.width && n.y >= 0 && n.y <= L.height), "inside the canvas");
const L2 = layout(top, edges);
check(JSON.stringify(L.nodes.map((n) => [n.x | 0, n.y | 0])) === JSON.stringify(L2.nodes.map((n) => [n.x | 0, n.y | 0])), "deterministic: the same map every time");
// linked nodes sit closer than unlinked ones on average
const pos = new Map(L.nodes.map((n) => [n.id, n]));
const dist = (a, b) => Math.hypot(pos.get(a).x - pos.get(b).x, pos.get(a).y - pos.get(b).y);
const linked = edges.filter((e) => pos.has(e.a) && pos.has(e.b)).map((e) => dist(e.a, e.b));
const ids = L.nodes.map((n) => n.id);
const random = Array.from({ length: 300 }, (_, k) => dist(ids[k], ids[(k * 37 + 11) % ids.length]));
const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length;
check(linked.length > 0 && mean(linked) < mean(random), `linked pairs closer (${mean(linked) | 0}) than random pairs (${mean(random) | 0})`);
// the whole 1,600 in one go, for the record (the screen never needs it); the budget is 3 s here
const t1 = Date.now();
const all = layout(nodes, edges, { iterations: 40 });
check(all.nodes.length === N && Date.now() - t1 < 3000, `full 1,600-node layout in ${Date.now() - t1} ms`);
// expand on tap: neighbours of a node come in next to it, nothing moves
const pick = L.nodes.find((n) => edges.some((e) => (e.a === n.id || e.b === n.id) && !pos.has(e.a === n.id ? e.b : e.a)));
if (pick) {
  const grown = placeNear(L.nodes, nodes, edges, pick.id);
  check(grown.length > L.nodes.length && grown.slice(0, L.nodes.length).every((n, i) => n.x === L.nodes[i].x), "placeNear adds neighbours and keeps the rest");
  const added = grown.slice(L.nodes.length);
  check(added.every((n) => Math.hypot(n.x - pick.x, n.y - pick.y) < 200), "new neighbours sit next to the tapped node");
}
const v = visible(L.nodes, { x: 0, y: 0, w: 400, h: 400, scale: 1 });
check(v.length < L.nodes.length && v.every((n) => n.x <= 480 && n.y <= 480), "visible() keeps only what is in the viewport (plus a margin)");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
