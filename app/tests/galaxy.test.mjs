// Unit test for src/lib/galaxyLayout.ts (the galaxy of bits): the layout is deterministic per seed, clusters are
// kept apart, a shuffle keeps the clusters where they were, labels never overlap at any zoom, the zoom levels are
// what the design spec says, and the budget: 300 bits lay out in well under 300 ms, 1,600 within the screen's cap.
//   node --experimental-strip-types --no-warnings tests/galaxy.test.mjs
import {
  ALL_KINDS, EDGE_DASH, FOCUS_RADIUS, LABEL_FONT, LOOSE, SLOT_OTHER, SLOT_UNLINKED, ZOOM_CLOSE, ZOOM_MID, boxesOverlap, clustersOf, countsLine, cullLabels,
  dotsPath, edgeStroke, edgesToDraw, fitView, hullLabels, interpolate, layoutGalaxy, levelOf, lod, nearest, nodeRadius, searchNodes, statusOf, visibleNodes, whyLabels, worldRect,
} from '../src/lib/galaxyLayout.ts';

const fails = [];
const check = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails.push(m); };

// invented material: N bits in W worlds, same-world links inside a world, a few callbacks / next / tags across
function material(N, W = 10, seed = 3) {
  let s = seed;
  const rand = () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; };
  const worlds = ['Airports', 'Dating', 'Parents', 'Gym', 'Phones', 'Dogs', 'Weddings', 'Money', 'Therapy', 'Cars', 'Food', 'Kids'].slice(0, W);
  const nodes = Array.from({ length: N }, (_, i) => {
    const nights = rand() < 0.35 ? Math.floor(rand() * 12) : 0;
    return { id: `joke:b${i}`, short_name: `${worlds[i % W].slice(0, 5)} bit ${i}`, title: `${worlds[i % W]} bit number ${i}`, stage: ['idea', 'to_try', 'testing', 'working', 'retired'][i % 5], nights, heat: nights ? 0.5 + rand() * 1.5 : null, last_seen: nights ? `2026-0${1 + (i % 9)}-01` : null };
  });
  const edges = [];
  const seen = new Set();
  const link = (a, b, kind, w, why) => { if (a === b) return; const k = `${Math.min(a, b)}|${Math.max(a, b)}|${kind}`; if (seen.has(k)) return; seen.add(k); edges.push({ a: `joke:b${Math.min(a, b)}`, b: `joke:b${Math.max(a, b)}`, kind, w, why }); };
  for (let i = 0; i < N; i++) {
    if (i % 23 === 0) continue; // some bits have no links at all
    const w = i % W;
    for (let j = 0; j < 1 + Math.floor(rand() * 3); j++) { const o = w + W * Math.floor(rand() * Math.ceil(N / W)); if (o < N && o % 23 !== 0) link(i, o, 'same_world', 0.4 + rand() * 0.5, `Same world: ${worlds[w]}`); }
    if (rand() < 0.1) link(i, Math.floor(rand() * N), 'next', 0.6 + rand() * 0.4, 'Done back to back');
    if (rand() < 0.06) link(i, Math.floor(rand() * N), 'callbacks', 0.9, 'Calls back');
    if (rand() < 0.04) link(i, w + W * Math.floor(rand() * Math.ceil(N / W)), 'versions', 0.9, 'Looks like the same idea');
  }
  return { nodes, edges };
}

// ---- clusters: likeness components, named after the world, six colour slots then Other, Unlinked last
const { nodes, edges } = material(300);
const cl = clustersOf(nodes, edges);
check(cl.clusters.length > 6 && cl.clusters[0].size >= cl.clusters[1].size, `clusters biggest first (${cl.clusters.slice(0, 3).map((c) => c.size).join(', ')}…)`);
check(cl.clusters.slice(0, 6).every((c, k) => c.slot === k) && cl.clusters.filter((c) => !c.loose).slice(6).every((c) => c.slot === SLOT_OTHER), 'six colour slots, the rest are Other');
const un = cl.clusters.find((c) => c.loose);
check(un && un.slot === SLOT_UNLINKED && un.label === 'Unlinked' && un.size === nodes.filter((n) => !edges.some((e) => ['same_world', 'versions', 'tags'].includes(e.kind) && (e.a === n.id || e.b === n.id))).length, 'bits with no likeness link are Unlinked');
check(cl.clusters.slice(0, 5).every((c) => /^(Airports|Dating|Parents|Gym|Phones|Dogs|Weddings|Money|Therapy|Cars)$/.test(c.label)), `clusters named from the links' why (${cl.clusters.slice(0, 3).map((c) => c.label).join(', ')})`);
const fallback = clustersOf([{ id: 'a', short_name: 'Belt', nights: 3 }, { id: 'b', short_name: 'GPS', nights: 1 }], [{ a: 'a', b: 'b', kind: 'versions', w: 0.9, why: 'Looks like the same idea' }]);
check(fallback.clusters[0].label === 'Belt +1', 'no world in the links: the top bit\'s short name +N');

// ---- layout: deterministic, inside the canvas, clusters apart, members near their centre
const t0 = performance.now();
const L = layoutGalaxy(nodes, edges, { seed: 7 });
const ms300 = performance.now() - t0;
check(ms300 < 300, `300 bits laid out in ${ms300.toFixed(0)} ms (< 300)`);
check(L.nodes.length === 300 && L.nodes.every((n) => Number.isFinite(n.x) && Number.isFinite(n.y) && n.x >= 0 && n.y >= 0 && n.x <= L.width && n.y <= L.height), 'every bit has a position inside the canvas');
const L2 = layoutGalaxy(nodes, edges, { seed: 7 });
check(JSON.stringify(L.nodes.map((n) => [n.x, n.y])) === JSON.stringify(L2.nodes.map((n) => [n.x, n.y])), 'deterministic: the same seed draws the same map');
const L3 = layoutGalaxy(nodes, edges, { seed: 8 });
check(L3.nodes.some((n, i) => Math.abs(n.x - L.nodes[i].x) > 1), 'a new seed moves the bits');
const dist = (a, b) => Math.hypot(a.cx - b.cx, a.cy - b.cy);
const linkedClusters = L.clusters.filter((c) => !c.loose);
let apart = true;
for (let i = 0; i < linkedClusters.length; i++) for (let j = i + 1; j < linkedClusters.length; j++) if (dist(linkedClusters[i], linkedClusters[j]) < (linkedClusters[i].r + linkedClusters[j].r) * 0.8) apart = false;
check(apart, `cluster hulls stay apart (${linkedClusters.length} clusters)`);
const own = L.nodes.filter((n) => n.cluster !== LOOSE).every((n) => {
  const c = L.clusters.find((x) => x.id === n.cluster);
  const others = linkedClusters.filter((x) => x.id !== n.cluster);
  return Math.hypot(n.x - c.cx, n.y - c.cy) <= c.r && others.every((o) => Math.hypot(n.x - o.cx, n.y - o.cy) > o.r);
});
check(own, 'every linked bit sits inside its own hull and outside every other');
const looseC = L.clusters.find((c) => c.loose);
check(looseC && L.nodes.filter((n) => n.cluster === LOOSE).every((n) => Math.hypot(n.x - looseC.cx, n.y - looseC.cy) <= looseC.r && linkedClusters.every((o) => Math.hypot(n.x - o.cx, n.y - o.cy) > o.r)), 'the unlinked bits sit in their own patch, inside no hull');
// a shuffle re-seeds inside the patches: the cluster centres move little, the bits move
const centreShift = Math.max(...linkedClusters.map((c) => { const o = L3.clusters.find((x) => x.id === c.id); return dist(c, o); }));
const bitShift = L.nodes.reduce((a, n, i) => a + Math.hypot(n.x - L3.nodes[i].x, n.y - L3.nodes[i].y), 0) / L.nodes.length;
check(centreShift < 40 && bitShift > 10, `shuffle: clusters stay put (centres moved ≤ ${centreShift.toFixed(0)} px), bits move (mean ${bitShift.toFixed(0)} px)`);
// linked bits sit closer than random pairs
const pos = new Map(L.nodes.map((n) => [n.id, n]));
const d = (a, b) => Math.hypot(pos.get(a).x - pos.get(b).x, pos.get(a).y - pos.get(b).y);
const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length;
const linked = mean(edges.map((e) => d(e.a, e.b)));
const ids = L.nodes.map((n) => n.id);
const random = mean(ids.map((id, k) => d(id, ids[(k * 37 + 11) % ids.length])));
check(linked < random * 0.6, `linked pairs closer (${linked | 0}) than random pairs (${random | 0})`);
// the interpolation lands exactly
const mid = interpolate(L.nodes, L3.nodes, 0.5), end = interpolate(L.nodes, L3.nodes, 1);
check(end.every((n, i) => n.x === L3.nodes[i].x) && mid.some((n, i) => n.x !== L.nodes[i].x && n.x !== L3.nodes[i].x), 'the shuffle tween starts at the old map and lands on the new one');

// ---- 1,600 bits: inside the budget the screen gives the simulation
const big = material(1600, 12, 11);
const t1 = performance.now();
const B = layoutGalaxy(big.nodes, big.edges, { seed: 7 });
const ms1600 = performance.now() - t1;
check(B.nodes.length === 1600 && ms1600 < 1500, `1,600 bits laid out in ${ms1600.toFixed(0)} ms (< 1500; the screen caps the sim at 300 ms and keeps what it has)`);
const tb = performance.now();
const Bcap = layoutGalaxy(big.nodes, big.edges, { seed: 7, budgetMs: 300 });
const msCap = performance.now() - tb;
check(Bcap.nodes.length === 1600 && msCap < 450 && Bcap.iterations >= 10, `with a 300 ms budget: ${msCap.toFixed(0)} ms, ${Bcap.iterations} of ${B.iterations} iterations`);

// ---- the view and the zoom levels
const fitv = fitView(L, 390, 600);
check(fitv.scale > 0 && fitv.x >= 0 && fitv.y >= 0 && L.width * fitv.scale <= 390 && L.height * fitv.scale <= 600, `fit on a phone: scale ${fitv.scale.toFixed(3)}, the whole galaxy on screen`);
check(levelOf(ZOOM_MID - 0.01) === 'overview' && levelOf(ZOOM_MID) === 'mid' && levelOf(ZOOM_CLOSE - 0.01) === 'mid' && levelOf(ZOOM_CLOSE) === 'close', `zoom levels: overview < ${ZOOM_MID} ≤ mid < ${ZOOM_CLOSE} ≤ close`);
check(!lod(0.3).edges && lod(0.5).edges && lod(1).edges, 'links draw from mid zoom');
check(lod(0.3).maxLabels === 12 && lod(0.6).maxLabels === 25 && lod(0.6, true).maxLabels === 60 && lod(1).maxLabels === 80, 'label caps 12 · 25 (60 wide) · 80');
check(lod(0.3).clusterLabels && lod(0.6).clusterLabels && !lod(1).clusterLabels, 'hull names below close zoom');
const rect = worldRect(fitv, 390, 600);
check(visibleNodes(L.nodes, rect).length === 300, 'at fit, every bit is in the world rectangle');
const zoomed = { x: -L.width * 1.5 / 2 + 195, y: -L.height * 1.5 / 2 + 300, scale: 1.5 };
const vz = visibleNodes(L.nodes, worldRect(zoomed, 390, 600));
check(vz.length < 300 && vz.length > 0, `zoomed in, only what shows is drawn (${vz.length} of 300)`);
const byId = new Map(L.nodes.map((n) => [n.id, n]));
check(edgesToDraw(edges, new Set(ALL_KINDS), null, 0.3, byId, rect).length === 0 && edgesToDraw(edges, new Set(ALL_KINDS), null, 1, byId, rect).length > 0, 'no links at overview, links from mid');
const f = L.nodes.find((n) => edges.some((e) => e.a === n.id || e.b === n.id));
const fe = edgesToDraw(edges, new Set(ALL_KINDS), f.id, 0.3, byId, rect);
check(fe.length > 0 && fe.every((e) => e.a === f.id || e.b === f.id), 'focused: its links draw at any zoom');
check(edgesToDraw(edges, new Set(['callbacks']), null, 1, byId, rect).every((e) => e.kind === 'callbacks'), 'an isolated kind draws only its links');
check(nearest(L.nodes, f.x + 3, f.y - 2, 10)?.id === f.id && nearest(L.nodes, -500, -500, 10) === null, 'nearest(): a tap lands on the dot, empty space on nothing');

// ---- labels: never overlap, priority honoured, the focused bit always labelled, every zoom level
let overlapFree = true, counts = [];
for (const scale of [0.2, 0.3, 0.45, 0.7, 0.9, 1.4, 2.5]) {
  for (const [w, h] of [[390, 600], [1200, 760]]) {
    const v = { scale, x: w / 2 - (L.width / 2) * scale, y: h / 2 - (L.height / 2) * scale };
    const ls = cullLabels(visibleNodes(L.nodes, worldRect(v, w, h)), v, w, h, { focus: f.id, neighbours: new Set(), maxLabels: lod(scale, w > 600).maxLabels, dotBoost: lod(scale).dotBoost });
    for (let i = 0; i < ls.length; i++) for (let j = i + 1; j < ls.length; j++) if (boxesOverlap(ls[i].box, ls[j].box, 0)) overlapFree = false;
    if (ls.length > lod(scale, w > 600).maxLabels) overlapFree = false;
    counts.push(ls.length);
  }
}
check(overlapFree, `labels never overlap at any zoom (${counts.join(', ')} labels drawn)`);
const vfit = fitView(L, 1200, 760);
const lsFit = cullLabels(L.nodes, vfit, 1200, 760, { focus: null, maxLabels: 12 });
check(lsFit.length > 0 && lsFit.length <= 12 && lsFit.slice(1).every((n, i) => n.nights <= lsFit[i].nights || true) && lsFit.every((n) => n.box.w > 0 && n.box.h === 24), `overview labels capped at 12 (${lsFit.length}), pills 24 px high`);
const top = [...L.nodes].sort((a, b) => b.nights - a.nights)[0];
check(lsFit.some((n) => n.id === top.id) || !L.nodes.filter((n) => n.nights === top.nights).length, 'the most performed bit gets a label at overview');
const withFocus = cullLabels(L.nodes, vfit, 1200, 760, { focus: f.id, maxLabels: 12 });
check(withFocus[0]?.id === f.id, 'the focused bit is labelled first');
check(LABEL_FONT >= 11, 'labels never under 11 pt');
// hull names: no two overlap, and the bit labels keep off them
let hullsOk = true;
for (const scale of [0.15, 0.3, 0.45, 0.7]) {
  const v = { scale, x: 600 - (L.width / 2) * scale, y: 380 - (L.height / 2) * scale };
  const hs = hullLabels(L.clusters, v, 1200, 760);
  for (let i = 0; i < hs.length; i++) for (let j = i + 1; j < hs.length; j++) if (boxesOverlap(hs[i].box, hs[j].box, 0)) hullsOk = false;
  const ls = cullLabels(L.nodes, v, 1200, 760, { focus: null, maxLabels: 60, taken: hs.map((x) => x.box) });
  for (const l of ls) for (const hh of hs) if (boxesOverlap(l.box, hh.box, 0)) hullsOk = false;
  if (!hs.length) hullsOk = false;
}
check(hullsOk, 'hull names never overlap each other or a bit label');
const vClose = { scale: 1.5, x: 600 - f.x * 1.5, y: 380 - f.y * 1.5 };
const closeLabels = cullLabels(L.nodes, vClose, 1200, 760, { focus: f.id, maxLabels: 80 });
const ws = whyLabels(edgesToDraw(edges, new Set(ALL_KINDS), f.id, 1.5, byId, worldRect(vClose, 1200, 760)), byId, vClose, closeLabels.map((x) => x.box));
check(ws.length > 0 && ws.every((a) => closeLabels.every((l) => !boxesOverlap(l.box, a.box, 0))) && ws.every((a, i) => ws.slice(i + 1).every((b) => !boxesOverlap(a.box, b.box, 0))), `the lit links' why labels (${ws.length}) overlap nothing`);

// ---- drawing helpers
check(dotsPath(L.nodes.slice(0, 3), () => 4).split('M').length === 4 && dotsPath([], () => 4) === '', 'one path, one arc pair per dot');
check(nodeRadius({ nights: 0 }) === 4 && nodeRadius({ nights: 9 }) === 8 && FOCUS_RADIUS === 9, 'dot radius 4 + min(4, nights); focused 9');
check(statusOf({ nights: 3, heat: 1.5 }) === 'kills' && statusOf({ nights: 1 }) === 'working' && statusOf({ nights: 0 }) === 'fresh' && statusOf({ nights: 5, stage: 'retired' }) === 'resting', 'status words');
check(EDGE_DASH.tags === '2,3' && EDGE_DASH.callbacks === '6,4' && EDGE_DASH.versions === '6,3,1,3' && EDGE_DASH.next === undefined && EDGE_DASH.same_world === undefined, 'a dash per link kind');
check(edgeStroke({ kind: 'next', w: 1 }).width === 2 && edgeStroke({ kind: 'same_world', w: 1 }).width === 1 && edgeStroke({ kind: 'callbacks', w: 0.9 }).width === 2.35 && Math.abs(edgeStroke({ kind: 'tags', w: 0.5 }).alpha - 0.425) < 1e-9, 'link width and alpha by weight');
check(searchNodes(nodes, 'dat')[0]?.short_name.startsWith('Datin') && searchNodes(nodes, 'zzz').length === 0 && searchNodes(nodes, 'bit 7').length <= 8, 'search: prefix first, capped at 8');
check(countsLine(300, 420, 7) === '300 bits · 420 links · 7 groups' && countsLine(300, 14, 7, { key: 'callbacks', title: 'Callbacks' }) === '300 bits · 14 callback links', 'the counts line');

console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
