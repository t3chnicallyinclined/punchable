// Tiny force-directed layout + SVG renderer. No libraries, works offline.
"use strict";

const DeskGraph = {};

// Deterministic PRNG so the same neighbourhood always lays out the same way.
DeskGraph.rng = function (seed) {
  let s = 2166136261;
  for (const c of String(seed)) s = Math.imul(s ^ c.charCodeAt(0), 16777619);
  return () => { s = Math.imul(s ^ (s >>> 15), 2246822507); s = Math.imul(s ^ (s >>> 13), 3266489909); return ((s ^= s >>> 16) >>> 0) / 4294967296; };
};

DeskGraph.layout = function (nodes, edges, centerId, iterations = 320) {
  const rnd = DeskGraph.rng(centerId);
  const byId = new Map();
  const hopCount = {};
  nodes.forEach((n) => { hopCount[n.hop] = (hopCount[n.hop] || 0) + 1; });
  const hopSeen = {};
  nodes.forEach((n) => {
    const k = (hopSeen[n.hop] = (hopSeen[n.hop] || 0) + 1);
    const a = (k / (hopCount[n.hop] || 1)) * Math.PI * 2 + rnd() * 0.4;
    const r = n.id === centerId ? 0 : 120 * n.hop + rnd() * 30;
    const p = { n, x: Math.cos(a) * r, y: Math.sin(a) * r, vx: 0, vy: 0 };
    byId.set(n.id, p);
  });
  const P = [...byId.values()];
  const L = edges.map((e) => [byId.get(e.src), byId.get(e.dst)]).filter(([a, b]) => a && b);
  const k = 90;
  let temp = 40;
  for (let it = 0; it < iterations; it++) {
    for (const p of P) { p.fx = 0; p.fy = 0; }
    for (let i = 0; i < P.length; i++) {
      for (let j = i + 1; j < P.length; j++) {
        const a = P[i], b = P[j];
        let dx = a.x - b.x, dy = a.y - b.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) { dx = rnd() - 0.5; dy = rnd() - 0.5; d2 = 0.5; }
        const d = Math.sqrt(d2);
        const f = (k * k) / d;
        a.fx += (dx / d) * f; a.fy += (dy / d) * f;
        b.fx -= (dx / d) * f; b.fy -= (dy / d) * f;
      }
    }
    for (const [a, b] of L) {
      const dx = a.x - b.x, dy = a.y - b.y;
      const d = Math.sqrt(dx * dx + dy * dy) || 0.01;
      const f = (d * d) / k;
      a.fx -= (dx / d) * f; a.fy -= (dy / d) * f;
      b.fx += (dx / d) * f; b.fy += (dy / d) * f;
    }
    for (const p of P) {
      p.fx -= p.x * 0.02; p.fy -= p.y * 0.02; // gentle gravity
      if (p.n.id === centerId) { p.x = 0; p.y = 0; continue; }
      const f = Math.sqrt(p.fx * p.fx + p.fy * p.fy) || 1;
      const step = Math.min(f, temp);
      p.x += (p.fx / f) * step; p.y += (p.fy / f) * step;
    }
    temp = Math.max(1, temp * 0.985);
  }
  return byId;
};

DeskGraph.render = function (svg, data, onTap) {
  const NS = "http://www.w3.org/2000/svg";
  const pos = DeskGraph.layout(data.nodes, data.edges, data.center);
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const p of pos.values()) { minX = Math.min(minX, p.x); maxX = Math.max(maxX, p.x); minY = Math.min(minY, p.y); maxY = Math.max(maxY, p.y); }
  const pad = 70;
  const w = Math.max(240, maxX - minX + pad * 2), h = Math.max(240, maxY - minY + pad * 2);
  svg.setAttribute("viewBox", `${minX - pad - (w - (maxX - minX + pad * 2)) / 2} ${minY - pad - (h - (maxY - minY + pad * 2)) / 2} ${w} ${h}`);
  svg.innerHTML = "";
  const el = (tag, attrs, parent = svg) => { const e = document.createElementNS(NS, tag); for (const [a, v] of Object.entries(attrs)) e.setAttribute(a, v); parent.appendChild(e); return e; };
  const COL = { confirmed: "#5fd08a", proposed: "#6aa8ff", rejected: "#ff6b6b" };
  const gE = el("g", {});
  for (const e of data.edges) {
    const a = pos.get(e.src), b = pos.get(e.dst);
    if (!a || !b) continue;
    const line = el("line", { x1: a.x, y1: a.y, x2: b.x, y2: b.y, stroke: COL[e.bucket] || "#888", "stroke-width": e.bucket === "confirmed" ? 2.5 : 1.6,
      "stroke-dasharray": e.bucket === "proposed" ? "6 4" : e.bucket === "rejected" ? "2 4" : "", opacity: e.bucket === "rejected" ? 0.6 : 0.85, class: "e" }, gE);
    el("title", {}, line).textContent = `${e.kind} · ${e.bucket}${e.confidence != null ? " · " + Math.round(e.confidence * 100) + "%" : ""}`;
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
    if (data.edges.length <= 40) { const t = el("text", { x: mx, y: my, "text-anchor": "middle", class: "ek", "font-size": "10", fill: "#9a9ea8" }, gE); t.textContent = e.kind; }
  }
  const gN = el("g", {});
  for (const p of pos.values()) {
    const n = p.n;
    const g = el("g", { class: "n", transform: `translate(${p.x},${p.y})`, tabindex: "0", role: "link", "data-id": n.id, "aria-label": `${n.type}: ${n.title}` }, gN);
    const center = n.id === data.center;
    if (n.type === "label") {
      el("rect", { x: -9, y: -9, width: 18, height: 18, rx: 4, transform: "rotate(45)", fill: "#c59cff", stroke: center ? "#fff" : "#0c0d0f", "stroke-width": 2 }, g);
    } else if (n.type === "joke") {
      el("circle", { r: center ? 15 : 11, fill: "#f5b544", stroke: center ? "#fff" : "#0c0d0f", "stroke-width": center ? 3 : 2 }, g);
    } else if (n.type === "performance") {
      el("rect", { x: -6, y: -6, width: 12, height: 12, fill: "#ff4f8b", stroke: "#0c0d0f", "stroke-width": 2 }, g);
    } else {
      el("circle", { r: 6, fill: "#9a9ea8", stroke: "#0c0d0f", "stroke-width": 2 }, g);
    }
    // big invisible hit target for fingers
    el("circle", { r: 22, fill: "transparent" }, g);
    const label = (n.title || n.id).length > 26 ? (n.title || n.id).slice(0, 25) + "…" : n.title || n.id;
    const t = el("text", { y: center ? 30 : 25, "text-anchor": "middle", "font-weight": center ? "700" : "400" }, g);
    t.textContent = label;
    el("title", {}, g).textContent = `${n.type}: ${n.title}`;
    const go = () => onTap(n);
    g.addEventListener("click", go);
    g.addEventListener("keydown", (ev) => { if (ev.key === "Enter") go(); });
  }
  return pos;
};
