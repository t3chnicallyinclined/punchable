// The galaxy of bits (Bits › Map): the pure half, no React Native, so tests/galaxy.test.mjs runs it in Node (named galaxyLayout: Galaxy.tsx is the screen; the disk is case-insensitive).
//  - clustersOf(): which bits are alike. A cluster is a connected component over the likeness links /api/map
//    returns (same world · versions · tags; the union-find allmap.ts uses), named after the world its links name
//    most often; the six biggest get a colour slot, the rest are "Other", bits with no likeness link "Unlinked".
//  - createSim() / layoutGalaxy(): a force-directed layout with one patch per cluster (packed on a spiral, biggest
//    first, independent of the seed), repulsion on a grid, springs on every link, gravity to the patch's centre.
//    Seeded: the same material draws the same map every open; "Shuffle" re-seeds inside the patches, so the
//    clusters stay where they were; the unlinked bits get a patch of their own at the edge, with no hull. The screen runs it in chunks off the render path (step()) and caches by rev.
//  - cullLabels(): the labels that fit at this zoom without overlapping, greedy by priority (the focused bit, its
//    neighbours, then nights · laughs · last seen). lod(): the three zoom levels (overview · mid · close).

export type GNode = { id: string; short_name: string; title?: string; stage?: string | null; nights: number; heat?: number | null; last_seen?: string | null; degree?: number };
export type GEdge = { a: string; b: string; kind: string; w: number; why?: string | null };
export type Placed = GNode & { x: number; y: number; cluster: number };
/** `slot`: 0-5 a palette colour, 6 = Other, 7 = Unlinked. */
export type Cluster = { id: number; label: string; size: number; slot: number; cx: number; cy: number; r: number; loose?: boolean };
export type GalaxyLayout = { nodes: Placed[]; clusters: Cluster[]; width: number; height: number; seed: number; iterations: number };
export type View = { x: number; y: number; scale: number };
export type Box = { x: number; y: number; w: number; h: number };
export type Status = 'kills' | 'working' | 'fresh' | 'resting';

export const SPACING = 30;        // the world unit: about one dot's breathing room
export const LOOSE = -1;          // the cluster id of bits with no likeness link
export const SLOT_OTHER = 6;
export const SLOT_UNLINKED = 7;
export const LIKENESS_KINDS = new Set(['same_world', 'versions', 'tags']);
export const ALL_KINDS = ['next', 'tags', 'same_world', 'callbacks', 'versions'];
/** The zoom levels (the design spec): overview below 0.45, close from 0.9. */
export const ZOOM_MID = 0.45;
export const ZOOM_CLOSE = 0.9;
export const MIN_SCALE = 0.25;
export const MAX_SCALE = 3;

/** Connected components over `edges` (union-find; the same as allmap.ts's, kept here so Node can run this file alone). */
export function components(ids: string[], edges: GEdge[]): Map<string, number> {
  const parent = new Map<string, string>(ids.map((i) => [i, i]));
  const find = (a: string): string => {
    let r = a;
    while (parent.get(r) !== r) r = parent.get(r)!;
    let c = a;
    while (parent.get(c) !== r) {
      const n = parent.get(c)!;
      parent.set(c, r);
      c = n;
    }
    return r;
  };
  for (const e of edges) {
    if (!parent.has(e.a) || !parent.has(e.b)) continue;
    const ra = find(e.a), rb = find(e.b);
    if (ra !== rb) parent.set(ra, rb);
  }
  const roots = new Map<string, number>();
  const out = new Map<string, number>();
  for (const i of ids) {
    const r = find(i);
    if (!roots.has(r)) roots.set(r, roots.size);
    out.set(i, roots.get(r)!);
  }
  return out;
}

/** A tiny deterministic PRNG (LCG) so a seed always draws the same map. */
export function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

export function statusOf(n: GNode): Status {
  if (n.stage === 'retired') return 'resting';
  if (n.nights >= 2 && (n.heat || 0) >= 1.3) return 'kills';
  if (n.nights > 0 || n.stage === 'working') return 'working';
  return 'fresh';
}
export const STATUS_WORD: Record<Status, string> = { kills: 'Kills', working: 'Working', fresh: 'Fresh', resting: 'Resting' };
export const STATUS_GLYPH: Record<Status, string> = { kills: '★', working: '●', fresh: '○', resting: '–' };

/** A dot's radius in world units: 4 + min(4, nights); the focused one is 9. */
export function nodeRadius(n: GNode): number {
  return 4 + Math.min(4, Math.max(0, n.nights));
}
export const FOCUS_RADIUS = 9;

/** Label order: nights, then laughs, then last seen, then the id (stable). Negative = `a` first. */
export function labelOrder(a: GNode, b: GNode): number {
  return b.nights - a.nights || (b.heat || 0) - (a.heat || 0) || (b.last_seen || '').localeCompare(a.last_seen || '') || a.id.localeCompare(b.id);
}

// ------------------------------------------------------------------ clusters
/** A cluster's name: the world its same-world links name most often, else its top bit's short name + " +N". */
export function clusterName(members: GNode[], edges: GEdge[], memberIds: Set<string>): string {
  const count = new Map<string, number>();
  for (const e of edges) {
    if (e.kind !== 'same_world' || !memberIds.has(e.a) || !memberIds.has(e.b)) continue;
    const m = /^Same (?:world|chunk): (.+)$/.exec(e.why || '');
    if (m) count.set(m[1], (count.get(m[1]) || 0) + 1);
  }
  let best: string | null = null, bn = 0;
  for (const [k, v] of count) if (v > bn || (v === bn && best != null && k < best)) { best = k; bn = v; }
  if (best) return best;
  const top = [...members].sort(labelOrder)[0];
  return top ? (members.length > 1 ? `${top.short_name} +${members.length - 1}` : top.short_name) : 'Bits';
}

/** Every bit's cluster id (LOOSE for no likeness link) and the clusters, biggest first, with their colour slots. */
export function clustersOf(nodes: GNode[], edges: GEdge[]): { of: Map<string, number>; clusters: Omit<Cluster, 'cx' | 'cy' | 'r'>[] } {
  const like = edges.filter((e) => LIKENESS_KINDS.has(e.kind));
  const ids = nodes.map((n) => n.id);
  const comp = components(ids, like);
  const linked = new Set<string>();
  for (const e of like) { linked.add(e.a); linked.add(e.b); }
  const members = new Map<number, GNode[]>();
  for (const n of nodes) {
    if (!linked.has(n.id)) continue;
    const c = comp.get(n.id)!;
    if (!members.has(c)) members.set(c, []);
    members.get(c)!.push(n);
  }
  const order = [...members.entries()].sort((a, b) => b[1].length - a[1].length || a[0] - b[0]);
  const of = new Map<string, number>();
  const clusters: Omit<Cluster, 'cx' | 'cy' | 'r'>[] = order.map(([c, ms], k) => {
    for (const n of ms) of.set(n.id, k);
    return { id: k, label: clusterName(ms, edges, new Set(ms.map((n) => n.id))), size: ms.length, slot: k < 6 ? k : SLOT_OTHER };
  });
  let loose = 0;
  for (const n of nodes) if (!linked.has(n.id)) { of.set(n.id, LOOSE); loose++; }
  if (loose) clusters.push({ id: LOOSE, label: 'Unlinked', size: loose, slot: SLOT_UNLINKED, loose: true });
  return { of, clusters };
}

// ------------------------------------------------------------------ the simulation
export type Sim = {
  /** Run up to `n` iterations; true when finished. */
  step: (n: number) => boolean;
  /** 0 … 1. */
  progress: () => number;
  result: () => GalaxyLayout;
  readonly total: number;
};

type Centre = Omit<Cluster, 'cx' | 'cy' | 'r'> & { x: number; y: number; r: number };

/** Pack the cluster patches on a sunflower spiral, biggest first, never overlapping. Seed-free: Shuffle keeps them. */
function packCentres(sizes: { id: number; size: number }[]): Map<number, { x: number; y: number; r: number }> {
  const out = new Map<number, { x: number; y: number; r: number }>();
  const placed: { x: number; y: number; r: number }[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  sizes.forEach(({ id, size }, k) => {
    const r = Math.max(SPACING * 2.2, Math.sqrt(size) * SPACING * 1.15) + SPACING;
    if (!placed.length) { out.set(id, { x: 0, y: 0, r }); placed.push({ x: 0, y: 0, r }); return; }
    for (let t = 1; t < 4000; t++) {
      const ang = k * golden + t * 0.07, rad = Math.sqrt(t) * SPACING * 0.9;
      const x = rad * Math.cos(ang), y = rad * Math.sin(ang);
      if (placed.every((p) => Math.hypot(p.x - x, p.y - y) >= p.r + r + SPACING * 0.5)) {
        out.set(id, { x, y, r });
        placed.push({ x, y, r });
        return;
      }
    }
    const fx = (placed.length + 1) * SPACING * 20;
    out.set(id, { x: fx, y: 0, r });
    placed.push({ x: fx, y: 0, r });
  });
  return out;
}

export function createSim(nodes: GNode[], edges: GEdge[], opts?: { seed?: number; iterations?: number }): Sim {
  const seed = opts?.seed ?? 7;
  const rand = rng(Math.imul(seed, 2654435761) ^ 0x9e3779b9);
  const N = nodes.length;
  const iterations = opts?.iterations ?? (N <= 400 ? 90 : N <= 900 ? 70 : 55);
  const idx = new Map(nodes.map((n, i) => [n.id, i]));
  const { of, clusters: cl } = clustersOf(nodes, edges);
  const members = new Map<number, number[]>();
  nodes.forEach((n, i) => {
    const c = of.get(n.id)!;
    if (!members.has(c)) members.set(c, []);
    members.get(c)!.push(i);
  });
  // every cluster gets a patch, the unlinked bits too (packed last, so they sit at the edge; no hull is drawn for them)
  const packed = packCentres(cl.map((c) => ({ id: c.id, size: c.size })));
  const centres: Centre[] = cl.map((c) => ({ ...c, ...packed.get(c.id)! }));
  const centreOf = new Map(centres.map((c) => [c.id, c]));
  // start positions: a jittered disc inside the patch
  const px = new Float64Array(N), py = new Float64Array(N);
  for (const [c, ms] of members) {
    const ce = centreOf.get(c)!;
    for (const i of ms) {
      const ang = rand() * 2 * Math.PI, r = (ce.r - SPACING) * Math.sqrt(rand()) * 0.9;
      px[i] = ce.x + r * Math.cos(ang);
      py[i] = ce.y + r * Math.sin(ang);
    }
  }
  const clusterOf = new Int32Array(N);
  nodes.forEach((n, i) => { clusterOf[i] = of.get(n.id)!; });
  const ea: number[] = [], eb: number[] = [], ew: number[] = [];
  for (const e of edges) {
    const a = idx.get(e.a), b = idx.get(e.b);
    if (a == null || b == null || a === b) continue;
    ea.push(a); eb.push(b); ew.push(0.5 + 0.5 * Math.max(0, Math.min(1, e.w)));
  }
  const k = SPACING;
  const cell = k * 2.5;
  let done = 0;
  let temp = k * 2.5;
  const dx = new Float64Array(N), dy = new Float64Array(N);

  function iterate() {
    dx.fill(0); dy.fill(0);
    // repulsion, only between nodes in neighbouring grid cells
    const grid = new Map<number, number[]>();
    for (let i = 0; i < N; i++) {
      const key = (Math.floor(px[i] / cell) * 73856093) ^ (Math.floor(py[i] / cell) * 19349663);
      let g = grid.get(key);
      if (!g) grid.set(key, (g = []));
      g.push(i);
    }
    for (let i = 0; i < N; i++) {
      const gx = Math.floor(px[i] / cell), gy = Math.floor(py[i] / cell);
      for (let ox = -1; ox <= 1; ox++) for (let oy = -1; oy <= 1; oy++) {
        const g = grid.get(((gx + ox) * 73856093) ^ ((gy + oy) * 19349663));
        if (!g) continue;
        for (const j of g) {
          if (j <= i) continue;
          let vx = px[i] - px[j], vy = py[i] - py[j];
          let d = Math.hypot(vx, vy);
          if (d < 0.01) { vx = rand() - 0.5; vy = rand() - 0.5; d = Math.hypot(vx, vy); }
          if (d > cell) continue;
          const f = Math.min(k * 3, (k * k) / d);
          vx = (vx / d) * f; vy = (vy / d) * f;
          dx[i] += vx; dy[i] += vy; dx[j] -= vx; dy[j] -= vy;
        }
      }
    }
    // springs on links (every kind: a callback pulls two bits together even across clusters)
    for (let e = 0; e < ea.length; e++) {
      const a = ea[e], b = eb[e];
      const vx = px[a] - px[b], vy = py[a] - py[b];
      const d = Math.hypot(vx, vy) || 0.01;
      const f = Math.min(k * 2, ((d * d) / k) * 0.06 * ew[e]);
      dx[a] -= (vx / d) * f; dy[a] -= (vy / d) * f; dx[b] += (vx / d) * f; dy[b] += (vy / d) * f;
    }
    // gravity to the patch's centre; a soft wall at its radius keeps the clusters apart
    for (let i = 0; i < N; i++) {
      const ce = centreOf.get(clusterOf[i])!;
      const vx = ce.x - px[i], vy = ce.y - py[i];
      const d = Math.hypot(vx, vy) || 0.01;
      const over = Math.max(0, d - (ce.r - SPACING));
      const f = d * 0.03 + over * 0.5;
      dx[i] += (vx / d) * f; dy[i] += (vy / d) * f;
    }
    for (let i = 0; i < N; i++) {
      const d = Math.hypot(dx[i], dy[i]) || 0.01;
      const m = Math.min(d, temp);
      px[i] += (dx[i] / d) * m;
      py[i] += (dy[i] / d) * m;
      // a hard wall just inside the patch: a callback across clusters bends toward the other bit, never leaves
      const ce = centreOf.get(clusterOf[i])!;
      const vx = px[i] - ce.x, vy = py[i] - ce.y, dd = Math.hypot(vx, vy), lim = ce.r - SPACING * 0.7;
      if (dd > lim) { px[i] = ce.x + (vx / dd) * lim; py[i] = ce.y + (vy / dd) * lim; }
    }
    temp = Math.max(k * 0.15, temp * 0.96);
    done++;
  }

  return {
    total: iterations,
    step(n) {
      for (let i = 0; i < n && done < iterations; i++) iterate();
      return done >= iterations;
    },
    progress: () => done / iterations,
    result() {
      // the hull: the patch's centre, out to its farthest member (never past the patch, so hulls never overlap)
      const clusters: Cluster[] = centres.map((ce) => {
        const ms = members.get(ce.id)!;
        const cx = ce.x, cy = ce.y;
        let r = 0;
        for (const i of ms) r = Math.max(r, Math.hypot(px[i] - cx, py[i] - cy) + nodeRadius(nodes[i]));
        return { id: ce.id, label: ce.label, size: ce.size, slot: ce.slot, loose: ce.loose, cx, cy, r: r + SPACING * 0.3 };
      });
      const pad = SPACING * 2;
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (let i = 0; i < N; i++) { minX = Math.min(minX, px[i]); minY = Math.min(minY, py[i]); maxX = Math.max(maxX, px[i]); maxY = Math.max(maxY, py[i]); }
      for (const c of clusters) { minX = Math.min(minX, c.cx - c.r); minY = Math.min(minY, c.cy - c.r); maxX = Math.max(maxX, c.cx + c.r); maxY = Math.max(maxY, c.cy + c.r); }
      if (!Number.isFinite(minX)) { minX = minY = 0; maxX = maxY = 1; }
      const sx = pad - minX, sy = pad - minY;
      for (const c of clusters) { c.cx += sx; c.cy += sy; }
      return {
        nodes: nodes.map((n, i) => ({ ...n, x: px[i] + sx, y: py[i] + sy, cluster: clusterOf[i] })),
        clusters, width: maxX - minX + pad * 2, height: maxY - minY + pad * 2, seed, iterations: done,
      };
    },
  };
}

/** Run the whole simulation now. `budgetMs` stops early on a slow device (the test leaves it off for determinism). */
export function layoutGalaxy(nodes: GNode[], edges: GEdge[], opts?: { seed?: number; iterations?: number; budgetMs?: number }): GalaxyLayout {
  const sim = createSim(nodes, edges, opts);
  const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
  const t0 = now();
  while (!sim.step(5)) if (opts?.budgetMs != null && now() - t0 > opts.budgetMs) break;
  return sim.result();
}

/** Positions on the way from layout `a` to layout `b` (same nodes, same order): the shuffle's gentle move. */
export function interpolate(a: Placed[], b: Placed[], t: number): Placed[] {
  const u = t <= 0 ? 0 : t >= 1 ? 1 : 1 - Math.pow(1 - t, 3); // ease out
  return b.map((n, i) => ({ ...n, x: a[i].x + (n.x - a[i].x) * u, y: a[i].y + (n.y - a[i].y) * u }));
}

// ------------------------------------------------------------------ the view
/** The view that shows the whole galaxy inside `w` × `h`, centred. */
export function fitView(L: { width: number; height: number }, w: number, h: number, pad = 24, maxScale = 1.2): View {
  const scale = Math.min(maxScale, Math.max(0.05, Math.min((w - pad * 2) / L.width, (h - pad * 2) / L.height)));
  return { scale, x: (w - L.width * scale) / 2, y: (h - L.height * scale) / 2 };
}

/** The view that puts a point in the middle at `scale`. */
export function centreOn(n: { x: number; y: number }, w: number, h: number, scale: number): View {
  return { scale, x: w / 2 - n.x * scale, y: h / 2 - n.y * scale };
}

/** Zoom by `factor` keeping the screen point (px, py) still, clamped. */
export function zoomAtPoint(v: View, factor: number, px: number, py: number, lo = MIN_SCALE, hi = MAX_SCALE): View {
  const s = Math.max(lo, Math.min(hi, v.scale * factor));
  const k = s / v.scale;
  return { scale: s, x: px - (px - v.x) * k, y: py - (py - v.y) * k };
}

export function lerpView(a: View, b: View, t: number): View {
  const u = t <= 0 ? 0 : t >= 1 ? 1 : 1 - Math.pow(1 - t, 3);
  return { x: a.x + (b.x - a.x) * u, y: a.y + (b.y - a.y) * u, scale: a.scale + (b.scale - a.scale) * u };
}

export type Level = 'overview' | 'mid' | 'close';
export function levelOf(scale: number): Level {
  return scale < ZOOM_MID ? 'overview' : scale < ZOOM_CLOSE ? 'mid' : 'close';
}

/** Level of detail at a zoom: links only from mid; label caps 12 · 25 (60 wide) · 80; hull names below close. */
export function lod(scale: number, wide = false): { level: Level; edges: boolean; maxLabels: number; clusterLabels: boolean; dotBoost: number } {
  const level = levelOf(scale);
  return {
    level,
    edges: level !== 'overview',
    maxLabels: level === 'overview' ? 12 : level === 'mid' ? (wide ? 60 : 25) : 80,
    clusterLabels: level !== 'close',
    dotBoost: scale < 1 ? Math.min(2.2, 1 / Math.sqrt(scale)) : 1, // dots keep a visible size when zoomed out
  };
}

/** A zoom level bucket: the dot sizes and strokes are re-drawn only when it changes. */
export function scaleBucket(scale: number): number {
  return Math.round(Math.log2(Math.max(0.03, scale)) * 3) / 3;
}

/** The world rectangle the viewport shows (plus a margin), snapped to a grid so a small pan re-uses the last set. */
export function worldRect(view: View, w: number, h: number, margin = 60, snap = 120): Box {
  const x0 = -view.x / view.scale - margin, y0 = -view.y / view.scale - margin;
  const x1 = x0 + w / view.scale + 2 * margin, y1 = y0 + h / view.scale + 2 * margin;
  const f = (v: number) => Math.floor(v / snap) * snap, c = (v: number) => Math.ceil(v / snap) * snap;
  return { x: f(x0), y: f(y0), w: c(x1) - f(x0), h: c(y1) - f(y0) };
}

export function inBox(n: { x: number; y: number }, b: Box): boolean {
  return n.x >= b.x && n.x <= b.x + b.w && n.y >= b.y && n.y <= b.y + b.h;
}

/** What is inside the world rectangle: the dots to draw. */
export function visibleNodes(nodes: Placed[], b: Box): Placed[] {
  return nodes.filter((n) => inBox(n, b));
}

/** The nearest bit to a world point within `radius` world units (a tap). */
export function nearest(nodes: Placed[], x: number, y: number, radius: number): Placed | null {
  let best: Placed | null = null, bd = radius * radius;
  for (const n of nodes) {
    const d = (n.x - x) * (n.x - x) + (n.y - y) * (n.y - y);
    if (d <= bd) { bd = d; best = n; }
  }
  return best;
}

// ------------------------------------------------------------------ labels
export const LABEL_FONT = 12;   // screen pt, never under 11 (HIG)
export const LABEL_H = 24;      // the pill's height in screen px
const CHAR_W = 0.58;            // Inter medium, roughly, per pt
export type Label = Placed & { sx: number; sy: number; box: Box };

/** The width of a label pill in screen px for `text`. */
export function labelWidth(text: string, font = LABEL_FONT): number {
  return Math.ceil(Math.min(22, text.length) * font * CHAR_W) + 18;
}

export function boxesOverlap(a: Box, b: Box, gap = 2): boolean {
  return a.x < b.x + b.w + gap && b.x < a.x + a.w + gap && a.y < b.y + b.h + gap && b.y < a.y + a.h + gap;
}

/**
 * The labels that fit: candidates in the viewport, ordered by priority (the focused bit, then its neighbours, then
 * `labelOrder`), placed to the right of the dot, each kept only if it overlaps none already placed; at most
 * `maxLabels`. Screen-space, so the result never overlaps at this zoom.
 */
export function cullLabels(nodes: Placed[], view: View, w: number, h: number, opts: { focus?: string | null; neighbours?: Set<string>; maxLabels: number; font?: number; dotBoost?: number; taken?: Box[] }): Label[] {
  const font = opts.font ?? LABEL_FONT;
  const boost = opts.dotBoost ?? 1;
  const tier = (n: Placed) => (n.id === opts.focus ? 2 : opts.neighbours?.has(n.id) ? 1 : 0);
  const cands: Placed[] = [];
  for (const n of nodes) {
    const sx = view.x + n.x * view.scale, sy = view.y + n.y * view.scale;
    if (sx < -40 || sy < -40 || sx > w + 40 || sy > h + 40) continue;
    cands.push(n);
  }
  cands.sort((a, b) => tier(b) - tier(a) || labelOrder(a, b));
  const out: Label[] = [];
  for (const n of cands) {
    if (out.length >= opts.maxLabels) break;
    const sx = view.x + n.x * view.scale, sy = view.y + n.y * view.scale;
    const r = (n.id === opts.focus ? FOCUS_RADIUS : nodeRadius(n)) * view.scale * boost;
    const box: Box = { x: sx + r + 3, y: sy - LABEL_H / 2, w: labelWidth(n.short_name, font), h: LABEL_H };
    if (box.x + box.w > w + 8 || box.y < 0 || box.y + box.h > h) continue;
    if (out.some((o) => boxesOverlap(o.box, box)) || opts.taken?.some((t) => boxesOverlap(t, box))) continue;
    out.push({ ...n, sx, sy, box });
  }
  return out;
}

export const HULL_FONT = 13;
export type HullLabel = { c: Cluster; box: Box };
/** The hull names ("Parents · 25") that fit: biggest cluster first, above its hull, none overlapping, none off screen. */
export function hullLabels(clusters: Cluster[], view: View, w: number, h: number): HullLabel[] {
  const out: HullLabel[] = [];
  for (const c of [...clusters].filter((x) => !x.loose).sort((a, b) => b.size - a.size || a.id - b.id)) {
    const bw = labelWidth(`${c.label} · ${c.size}`, HULL_FONT) - 6;
    const sx = view.x + c.cx * view.scale, sy = view.y + (c.cy - c.r) * view.scale - 22;
    const box: Box = { x: sx - bw / 2, y: sy, w: bw, h: 18 };
    if (box.x + box.w < 0 || box.x > w || box.y + box.h < 0 || box.y > h) continue;
    if (out.some((o) => boxesOverlap(o.box, box))) continue;
    out.push({ c, box });
  }
  return out;
}

/** The `why` of each lit link at its midpoint, kept only where it overlaps no label and no other why. */
export function whyLabels(edges: GEdge[], byId: Map<string, Placed>, view: View, taken: Box[], max = 12): { e: GEdge; mx: number; my: number; box: Box }[] {
  const out: { e: GEdge; mx: number; my: number; box: Box }[] = [];
  for (const e of edges) {
    const a = byId.get(e.a), b = byId.get(e.b);
    if (!a || !b || !e.why) continue;
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
    const bw = Math.min(180, labelWidth(e.why, 11));
    const box: Box = { x: view.x + mx * view.scale - bw / 2, y: view.y + my * view.scale - 8, w: bw, h: 16 };
    if (taken.some((t) => boxesOverlap(t, box)) || out.some((o) => boxesOverlap(o.box, box))) continue;
    out.push({ e, mx, my, box });
    if (out.length >= max) break;
  }
  return out;
}

/** One SVG path drawing a circle per node (one element instead of N; the dots layer of the galaxy). */
export function dotsPath(nodes: Placed[], radius: (n: Placed) => number): string {
  let d = '';
  for (const n of nodes) {
    const r = radius(n);
    d += `M${(n.x - r).toFixed(1)} ${n.y.toFixed(1)}a${r.toFixed(2)} ${r.toFixed(2)} 0 1 0 ${(2 * r).toFixed(2)} 0a${r.toFixed(2)} ${r.toFixed(2)} 0 1 0 ${(-2 * r).toFixed(2)} 0`;
  }
  return d;
}

/** One SVG path for a set of links. */
export function edgesPath(edges: GEdge[], byId: Map<string, Placed>): string {
  let d = '';
  for (const e of edges) {
    const a = byId.get(e.a), b = byId.get(e.b);
    if (!a || !b) continue;
    d += `M${a.x.toFixed(1)} ${a.y.toFixed(1)}L${b.x.toFixed(1)} ${b.y.toFixed(1)}`;
  }
  return d;
}

/** A link's stroke: width 1 + 1.5·w, alpha 0.25 + 0.35·w; the dash per kind so colour is never alone. */
export const EDGE_DASH: Record<string, string | undefined> = { next: undefined, same_world: undefined, tags: '2,3', callbacks: '6,4', versions: '6,3,1,3' };
export function edgeStroke(e: GEdge): { width: number; alpha: number } {
  const w = Math.max(0, Math.min(1, e.w));
  return { width: e.kind === 'next' ? 2 : e.kind === 'same_world' ? 1 : 1 + 1.5 * w, alpha: 0.25 + 0.35 * w };
}

/** The links to draw: the focused bit's, else (from mid zoom) every link with an end in the rectangle. */
export function edgesToDraw(edges: GEdge[], kinds: Set<string>, focus: string | null, scale: number, byId: Map<string, Placed>, rect: Box, cap = 1500): GEdge[] {
  const out: GEdge[] = [];
  if (focus) {
    for (const e of edges) if (kinds.has(e.kind) && (e.a === focus || e.b === focus)) out.push(e);
    return out;
  }
  if (!lod(scale).edges) return out;
  for (const e of edges) {
    if (!kinds.has(e.kind)) continue;
    const a = byId.get(e.a), b = byId.get(e.b);
    if (!a || !b || !(inBox(a, rect) || inBox(b, rect))) continue;
    out.push(e);
    if (out.length >= cap) break;
  }
  return out;
}

export function neighboursOf(edges: GEdge[], kinds: Set<string>, id: string | null): Set<string> {
  const s = new Set<string>();
  if (!id) return s;
  for (const e of edges) if (kinds.has(e.kind)) { if (e.a === id) s.add(e.b); else if (e.b === id) s.add(e.a); }
  return s;
}

/** Search: short name or title, word-prefix first, then anywhere; at most `limit`. */
export function searchNodes(nodes: GNode[], q: string, limit = 8): GNode[] {
  const s = q.trim().toLowerCase();
  if (!s) return [];
  const score = (n: GNode) => {
    const name = n.short_name.toLowerCase(), title = (n.title || '').toLowerCase();
    if (name.startsWith(s) || title.startsWith(s)) return 3;
    if (name.split(/\s+/).some((w) => w.startsWith(s)) || title.split(/\s+/).some((w) => w.startsWith(s))) return 2;
    if (name.includes(s) || title.includes(s)) return 1;
    return 0;
  };
  return nodes.map((n) => ({ n, sc: score(n) })).filter((x) => x.sc > 0).sort((a, b) => b.sc - a.sc || labelOrder(a.n, b.n)).slice(0, limit).map((x) => x.n);
}

/** The counts line: "N bits · M links · K groups", or with one link kind isolated "N bits · 14 callback links". */
export const KIND_NOUN: Record<string, string> = { next: 'next', tags: 'tag', same_world: 'same-world', callbacks: 'callback', versions: 'version' };
export function countsLine(nodes: number, links: number, groups: number, isolated?: { key: string; title: string } | null): string {
  if (isolated) return `${nodes} bits · ${links} ${KIND_NOUN[isolated.key] || isolated.title.toLowerCase()} link${links === 1 ? '' : 's'}`;
  return `${nodes} bits · ${links} link${links === 1 ? '' : 's'} · ${groups} group${groups === 1 ? '' : 's'}`;
}
