// The all-bits map's layout: pure (no React Native), so tests/allmap.test.mjs runs it in Node and times it.
// Nodes cluster by what links them (the per-joke map's groups: next · tags · same world · callbacks · versions);
// each connected component gets its own patch of the canvas, and a short force-directed pass settles the nodes
// inside it. Level of detail: the screen lays out the top N (by nights, laughs) and brings a node's neighbours in
// on tap (placeNear), so 1,600 bits never need a full layout at once.
export type MapNode = { id: string; short_name: string; title?: string; stage?: string | null; nights: number; heat?: number | null; degree?: number };
export type MapEdge = { a: string; b: string; kind: string; w: number; why?: string | null };
export type Placed = MapNode & { x: number; y: number; comp: number };
export type Layout = { nodes: Placed[]; width: number; height: number; comps: number };

/** A tiny deterministic PRNG so the same material draws the same map every time. */
function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

function components(ids: string[], edges: MapEdge[]): Map<string, number> {
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

/**
 * Lay `nodes` out with `edges` (edges to nodes not in the list are ignored). `iterations` of a Fruchterman-Reingold
 * pass inside each component's patch; components are packed on a grid, largest first. Returns pixel positions.
 */
export function layout(nodes: MapNode[], edges: MapEdge[], opts?: { iterations?: number; spacing?: number; seed?: number }): Layout {
  const it = opts?.iterations ?? 60;
  const sp = opts?.spacing ?? 96;
  const rand = rng(opts?.seed ?? 7);
  const ids = nodes.map((n) => n.id);
  const idx = new Map(ids.map((id, i) => [id, i]));
  const comp = components(ids, edges);
  const members = new Map<number, number[]>();
  nodes.forEach((n, i) => {
    const c = comp.get(n.id)!;
    if (!members.has(c)) members.set(c, []);
    members.get(c)!.push(i);
  });
  // a patch per component: a square whose side grows with sqrt(size); packed row by row, big ones first
  const order = [...members.entries()].sort((a, b) => b[1].length - a[1].length || a[0] - b[0]);
  const patches: { c: number; side: number; x: number; y: number }[] = [];
  const total = nodes.length;
  const rowW = Math.max(sp * 4, Math.sqrt(total) * sp * 1.9);
  let cx = 0, cy = 0, rowH = 0;
  for (const [c, ms] of order) {
    const side = Math.max(sp * 1.4, Math.sqrt(ms.length) * sp * 1.25);
    if (cx + side > rowW && cx > 0) {
      cx = 0;
      cy += rowH + sp * 0.6;
      rowH = 0;
    }
    patches.push({ c, side, x: cx, y: cy });
    cx += side + sp * 0.6;
    rowH = Math.max(rowH, side);
  }
  const width = Math.max(rowW, ...patches.map((p) => p.x + p.side));
  const height = cy + rowH;
  const pos = nodes.map(() => ({ x: 0, y: 0 }));
  const adj: number[][] = nodes.map(() => []);
  for (const e of edges) {
    const a = idx.get(e.a), b = idx.get(e.b);
    if (a == null || b == null || a === b) continue;
    adj[a].push(b);
    adj[b].push(a);
  }
  for (const p of patches) {
    const ms = members.get(p.c)!;
    const n = ms.length;
    // start on a ring (one node: the centre), then relax
    ms.forEach((i, k) => {
      const ang = (2 * Math.PI * k) / Math.max(1, n) + rand() * 0.3;
      const r = n === 1 ? 0 : (p.side / 2 - sp * 0.5) * (0.55 + 0.45 * rand());
      pos[i] = { x: p.x + p.side / 2 + r * Math.cos(ang), y: p.y + p.side / 2 + r * Math.sin(ang) };
    });
    if (n < 2) continue;
    const area = p.side * p.side;
    const kk = Math.sqrt(area / n) * 0.8;
    let temp = p.side / 8;
    const set = new Set(ms);
    for (let step = 0; step < it; step++) {
      const disp = ms.map(() => ({ x: 0, y: 0 }));
      for (let a = 0; a < n; a++) {
        const i = ms[a];
        for (let b = a + 1; b < n; b++) {
          const j = ms[b];
          let dx = pos[i].x - pos[j].x, dy = pos[i].y - pos[j].y;
          let d = Math.sqrt(dx * dx + dy * dy) || 0.01;
          if (d > kk * 4) continue; // far apart: nothing to push
          const f = (kk * kk) / d;
          dx /= d; dy /= d;
          disp[a].x += dx * f; disp[a].y += dy * f;
          disp[b].x -= dx * f; disp[b].y -= dy * f;
        }
        for (const j of adj[i]) {
          if (!set.has(j)) continue;
          let dx = pos[i].x - pos[j].x, dy = pos[i].y - pos[j].y;
          const d = Math.sqrt(dx * dx + dy * dy) || 0.01;
          const f = (d * d) / kk;
          dx /= d; dy /= d;
          disp[a].x -= dx * f; disp[a].y -= dy * f;
        }
      }
      for (let a = 0; a < n; a++) {
        const i = ms[a];
        const d = Math.sqrt(disp[a].x * disp[a].x + disp[a].y * disp[a].y) || 0.01;
        const m = Math.min(d, temp);
        pos[i].x = Math.max(p.x + sp * 0.4, Math.min(p.x + p.side - sp * 0.4, pos[i].x + (disp[a].x / d) * m));
        pos[i].y = Math.max(p.y + sp * 0.4, Math.min(p.y + p.side - sp * 0.4, pos[i].y + (disp[a].y / d) * m));
      }
      temp *= 0.94;
    }
  }
  return { nodes: nodes.map((n, i) => ({ ...n, x: pos[i].x, y: pos[i].y, comp: comp.get(n.id)! })), width, height, comps: members.size };
}

/** The top `n` by nights, then laughs, then the ones with links (what the screen lays out first). */
export function topNodes(nodes: MapNode[], n: number): MapNode[] {
  return [...nodes].sort((a, b) => b.nights - a.nights || (b.heat || 0) - (a.heat || 0) || (b.degree || 0) - (a.degree || 0) || a.id.localeCompare(b.id)).slice(0, n);
}

/** Bring a node's neighbours in around it (level of detail: expand on tap). Existing positions are kept. */
export function placeNear(current: Placed[], all: MapNode[], edges: MapEdge[], id: string, spacing = 96): Placed[] {
  const have = new Set(current.map((n) => n.id));
  const me = current.find((n) => n.id === id);
  if (!me) return current;
  const byId = new Map(all.map((n) => [n.id, n]));
  const add: string[] = [];
  for (const e of edges) {
    const o = e.a === id ? e.b : e.b === id ? e.a : null;
    if (o && !have.has(o) && byId.has(o)) add.push(o);
  }
  const uniq = [...new Set(add)].slice(0, 12);
  const out = [...current];
  uniq.forEach((o, k) => {
    const ang = (2 * Math.PI * k) / uniq.length - Math.PI / 2;
    const r = spacing * (1.2 + 0.25 * (k % 2));
    out.push({ ...byId.get(o)!, x: me.x + r * Math.cos(ang), y: me.y + r * Math.sin(ang), comp: me.comp });
  });
  return out;
}

/** What is inside the viewport (plus a margin), for drawing only what shows. */
export function visible(nodes: Placed[], view: { x: number; y: number; w: number; h: number; scale: number }, margin = 80): Placed[] {
  const x0 = -view.x / view.scale - margin, y0 = -view.y / view.scale - margin;
  const x1 = x0 + view.w / view.scale + 2 * margin, y1 = y0 + view.h / view.scale + 2 * margin;
  return nodes.filter((n) => n.x >= x0 && n.x <= x1 && n.y >= y0 && n.y <= y1);
}
