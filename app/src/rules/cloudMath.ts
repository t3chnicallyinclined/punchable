// The cloud's geometry (lib/Cloud.native.tsx on Skia, lib/Cloud.tsx on SVG; api-dev/cloud_api.py gives x, y, z in
// [-1, 1]): an orbit camera (yaw, pitch, a target point, a zoom), real perspective, depth into size and fade, the
// gestures' camera moves, the nearest dot to a tap, and which dots get their names. Pure: tests/cloudMath.test.mjs.
// EVERY function here carries 'worklet': the phone calls them on the UI thread (frame callback, picture, gestures) and
// a plain function there crashes the app ("[Worklets] Tried to synchronously call a Remote Function", the build-16
// crash; `wrapAngle` without it was the same bug in builds 17-18). tests/cloudMath.test.mjs pins the directive.
// Spec: docs/design/SPEC-2026-11-02-cloud-3d.md.
export type CloudNode = { id: string; short_name: string; title?: string; stage?: string | null; nights: number; heat?: number | null; touched?: string | null; x: number; y: number; z: number; cluster: number };
export type CloudCluster = { id: number; label: string; size: number; cx: number; cy: number; cz: number };
export type CloudMap = { rev?: string | null; nodes: CloudNode[]; clusters: CloudCluster[]; counts: { jokes: number; placed: number; with_vectors: number; k?: number } };

/** The camera: an orbit about the target (tx, ty, tz), `lz` = ln(zoom) so a zoom animates evenly. Home = zeros. */
export type Cam = { yaw: number; pitch: number; tx: number; ty: number; tz: number; lz: number };
export const HOME: Cam = { yaw: 0, pitch: 0, tx: 0, ty: 0, tz: 0, lz: 0 };

export const R_MIN = 2.6;
export const R_MAX = 8.5;
export const ALPHA_MIN = 0.3;
export const PAD = 28;
/** Camera distance from the target at zoom 1, in world units (the cloud is about [-1, 1]³). */
export const EYE = 4;
/** Pitch never passes ~69°: up stays up. */
export const PITCH_LIMIT = 1.2;
export const ZOOM_MIN = 0.6;
export const ZOOM_MAX = 12;
export const LZ_MIN = Math.log(ZOOM_MIN);
export const LZ_MAX = Math.log(ZOOM_MAX);
/** The target stays inside the cloud's box, so part of the cloud is always on screen (HIG/maps). */
export const TARGET_LIMIT = 1.2;
/** Dots nearer the camera than 25 % of its distance are not drawn; fully drawn from 60 %. */
export const NEAR_CULL = 0.25;
export const NEAR_FULL = 0.6;
/** Bit names appear from 2.5× and are full by 3.5×; at most 30, one per 160 × 24 pt cell. */
export const LABEL_FROM = 2.5;
export const LABEL_FULL = 3.5;
export const LABEL_MAX = 30;
export const CELL_W = 160;
export const CELL_H = 24;
/** A tap reaches a dot within 18 pt of its edge, in screen space. */
export const SLOP = 18;

export function clamp(v: number, lo: number, hi: number): number {
  'worklet';
  return v < lo ? lo : v > hi ? hi : v;
}
export function clampPitch(p: number): number {
  'worklet';
  return clamp(p, -PITCH_LIMIT, PITCH_LIMIT);
}
export function clampLz(lz: number): number {
  'worklet';
  return clamp(lz, LZ_MIN, LZ_MAX);
}
/** Half the padded square: the pixels per world unit at zoom 1 on the target plane. */
export function fitRadius(w: number, h: number, pad = PAD): number {
  'worklet';
  return Math.max(1, (Math.min(w, h) - pad * 2) / 2);
}
/** A drag across the canvas's width turns the cloud 180°. */
export function radPerPx(w: number): number {
  'worklet';
  return Math.PI / Math.max(1, w);
}

/** Keep the angle in [0, 2π) so a long spin never grows the float. */
export function wrapAngle(theta: number): number {
  'worklet';
  const t = theta % (2 * Math.PI);
  return t < 0 ? t + 2 * Math.PI : t;
}

/** One point through the camera: screen position, depth cue (1 = nearest), perspective growth and the near fade
 *  (0 = not drawn). */
export type Pt = { sx: number; sy: number; depth: number; grow: number; fade: number };
export function projectPoint(x: number, y: number, z: number, cam: Cam, w: number, h: number): Pt {
  'worklet';
  const R = fitRadius(w, h);
  const cy = Math.cos(cam.yaw), sy = Math.sin(cam.yaw), cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  const dist = EYE / Math.exp(cam.lz);
  const px = x - cam.tx, py = y - cam.ty, pz = z - cam.tz;
  const x1 = px * cy - pz * sy;
  const z1 = px * sy + pz * cy;
  const y2 = py * cp - z1 * sp;
  const z2 = py * sp + z1 * cp;
  const d = dist - z2;
  const rel = d / dist;
  const fade = clamp((rel - NEAR_CULL) / (NEAR_FULL - NEAR_CULL), 0, 1);
  const k = d > 1e-6 ? (R * EYE) / d : 0;
  return { sx: w / 2 + x1 * k, sy: h / 2 - y2 * k, depth: clamp((z2 + 1) / 2, 0, 1), grow: d > 1e-6 ? EYE / d : 0, fade };
}

/** The projection of every dot, in flat arrays (no object per dot on the UI thread). alpha 0 = not drawn. */
export type Proj = { sx: Float32Array; sy: Float32Array; depth: Float32Array; r: Float32Array; alpha: Float32Array; fade: Float32Array };
export function newProj(n: number): Proj {
  'worklet';
  return { sx: new Float32Array(n), sy: new Float32Array(n), depth: new Float32Array(n), r: new Float32Array(n), alpha: new Float32Array(n), fade: new Float32Array(n) };
}
/** Dot radius: depth sets it as before, the perspective grows it by √(growth), clamped, so the gaps between dots grow
 *  faster than the dots (a dense spot separates as you zoom). */
export function dotRadius(depth: number, grow: number): number {
  'worklet';
  return (R_MIN + (R_MAX - R_MIN) * depth) * clamp(Math.sqrt(grow), 0.75, 2.6);
}
export function projectInto(n: number, x: ArrayLike<number>, y: ArrayLike<number>, z: ArrayLike<number>, cam: Cam, w: number, h: number, out: Proj): void {
  'worklet';
  const R = fitRadius(w, h);
  const cyw = Math.cos(cam.yaw), syw = Math.sin(cam.yaw), cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  const dist = EYE / Math.exp(cam.lz);
  const ox = w / 2, oy = h / 2;
  for (let i = 0; i < n; i++) {
    const px = x[i] - cam.tx, py = y[i] - cam.ty, pz = z[i] - cam.tz;
    const x1 = px * cyw - pz * syw;
    const z1 = px * syw + pz * cyw;
    const y2 = py * cp - z1 * sp;
    const z2 = py * sp + z1 * cp;
    const d = dist - z2;
    const rel = d / dist;
    const fade = rel <= NEAR_CULL ? 0 : rel >= NEAR_FULL ? 1 : (rel - NEAR_CULL) / (NEAR_FULL - NEAR_CULL);
    const dep = z2 <= -1 ? 0 : z2 >= 1 ? 1 : (z2 + 1) / 2;
    out.depth[i] = dep;
    out.fade[i] = fade;
    if (fade <= 0) { out.alpha[i] = 0; out.sx[i] = -1e4; out.sy[i] = -1e4; out.r[i] = 0; continue; }
    const k = (R * EYE) / d;
    out.sx[i] = ox + x1 * k;
    out.sy[i] = oy - y2 * k;
    out.r[i] = dotRadius(dep, EYE / d);
    out.alpha[i] = (ALPHA_MIN + (1 - ALPHA_MIN) * dep) * fade;
  }
}

/** Far dots first so near ones draw on top: a counting sort into 16 depth buckets (no comparator calls). */
export function depthOrder(depth: Float32Array, n: number): Uint16Array {
  'worklet';
  const buckets = 16;
  const counts = new Uint16Array(buckets + 1);
  for (let i = 0; i < n; i++) counts[Math.min(buckets - 1, Math.floor(depth[i] * buckets)) + 1]++;
  for (let b = 0; b < buckets; b++) counts[b + 1] += counts[b];
  const sorted = new Uint16Array(n);
  for (let i = 0; i < n; i++) { const b = Math.min(buckets - 1, Math.floor(depth[i] * buckets)); sorted[counts[b]++] = i; }
  return sorted;
}

/** The drawn dot under a tap (screen space), or -1: nearest edge within `slop`; a dot not drawn is never hit. */
export function nearestIndex(p: Proj, n: number, px: number, py: number, slop = SLOP): number {
  'worklet';
  let best = -1, bd = Infinity;
  for (let i = 0; i < n; i++) {
    if (p.alpha[i] <= 0) continue;
    const dx = p.sx[i] - px, dy = p.sy[i] - py;
    const dd = Math.sqrt(dx * dx + dy * dy) - p.r[i];
    if (dd <= slop && (dd < bd || (dd === bd && best >= 0 && p.depth[i] > p.depth[best]))) { bd = dd; best = i; }
  }
  return best;
}

/** Drag: right turns the front right, down tilts the top toward you. */
export function orbitBy(cam: Cam, dx: number, dy: number, w: number): Cam {
  'worklet';
  const k = radPerPx(w);
  return { ...cam, yaw: cam.yaw - dx * k, pitch: clampPitch(cam.pitch + dy * k) };
}

/** The camera's right and up vectors in world space (the inverse of the yaw-then-pitch turn). */
function axes(cam: Cam): { rx: number; ry: number; rz: number; ux: number; uy: number; uz: number } {
  'worklet';
  const c = Math.cos(cam.yaw), s = Math.sin(cam.yaw), cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  return { rx: c, ry: 0, rz: -s, ux: -sp * s, uy: cp, uz: -sp * c };
}
function clampTarget(cam: Cam): Cam {
  'worklet';
  return { ...cam, tx: clamp(cam.tx, -TARGET_LIMIT, TARGET_LIMIT), ty: clamp(cam.ty, -TARGET_LIMIT, TARGET_LIMIT), tz: clamp(cam.tz, -TARGET_LIMIT, TARGET_LIMIT) };
}

/** Two fingers moving (dx, dy) screen points: the target plane follows them. */
export function panBy(cam: Cam, dx: number, dy: number, R: number): Cam {
  'worklet';
  const a = axes(cam);
  const u = 1 / (R * Math.exp(cam.lz));
  const mx = -dx * u, my = dy * u;
  return clampTarget({ ...cam, tx: cam.tx + a.rx * mx + a.ux * my, ty: cam.ty + a.ry * mx + a.uy * my, tz: cam.tz + a.rz * mx + a.uz * my });
}

/** Zoom to `lz1` about a screen point (ox, oy from the canvas centre): the point on the target plane under it stays. */
export function zoomAbout(cam: Cam, lz1: number, ox: number, oy: number, R: number): Cam {
  'worklet';
  const lz = clampLz(lz1);
  const a = axes(cam);
  const f = 1 / Math.exp(cam.lz) - 1 / Math.exp(lz);
  const wx = ox / R, wy = -oy / R;
  return clampTarget({ ...cam, lz, tx: cam.tx + (a.rx * wx + a.ux * wy) * f, ty: cam.ty + (a.ry * wx + a.uy * wy) * f, tz: cam.tz + (a.rz * wx + a.uz * wy) * f });
}

/** Where a double tap goes: 2× in about the point; at the limit, the whole map. */
export function doubleTapTarget(cam: Cam, ox: number, oy: number, R: number): Cam {
  'worklet';
  if (cam.lz >= LZ_MAX - 0.01) return { ...HOME, yaw: cam.yaw };
  return zoomAbout(cam, cam.lz + Math.LN2, ox, oy, R);
}

/** Whole map: the target home, zoom 1, upright; the yaw stays where he left it. */
export function homeFrom(cam: Cam): Cam {
  'worklet';
  return { ...HOME, yaw: cam.yaw };
}
export function isHome(cam: Cam): boolean {
  'worklet';
  return Math.abs(cam.pitch) < 0.02 && Math.abs(cam.lz) < 0.01 && Math.abs(cam.tx) + Math.abs(cam.ty) + Math.abs(cam.tz) < 0.01;
}

/** How far each group's members sit from its centre (RMS, world units), by cluster id. Once per payload. */
export function spreads(nodes: CloudNode[], clusters: CloudCluster[]): number[] {
  'worklet';
  const sum: number[] = [], cnt: number[] = [];
  const at: number[] = [];
  for (let k = 0; k < clusters.length; k++) at[clusters[k].id] = k;
  for (let k = 0; k < clusters.length; k++) { sum.push(0); cnt.push(0); }
  for (let i = 0; i < nodes.length; i++) {
    const k = at[nodes[i].cluster];
    if (k === undefined) continue;
    const c = clusters[k];
    const dx = nodes[i].x - c.cx, dy = nodes[i].y - c.cy, dz = nodes[i].z - c.cz;
    sum[k] += dx * dx + dy * dy + dz * dz; cnt[k]++;
  }
  return clusters.map((_, k) => (cnt[k] ? Math.sqrt(sum[k] / cnt[k]) : 0.3));
}

/** Fly to a group: the target its centre, the zoom framing its spread (1.6× to 6×); yaw and pitch stay. */
export function flyTo(cam: Cam, c: CloudCluster, spread: number): Cam {
  'worklet';
  const zoom = clamp(0.85 / Math.max(0.01, spread), 1.6, 6);
  return clampTarget({ ...cam, tx: c.cx, ty: c.cy, tz: c.cz, lz: Math.log(zoom) });
}

/** A group's name: depth and the near fade as before; past 2× the names away from the screen centre fade, so a
 *  zoomed view shows the group you are in. */
export function groupLabelAlpha(depth: number, fade: number, zoom: number, distPx: number, minDim: number): number {
  'worklet';
  const base = (0.35 + 0.65 * depth) * fade;
  if (zoom <= 2) return base;
  const t = clamp((zoom - 2) / 2, 0, 1);
  const keep = clamp(1 - distPx / (0.35 * Math.max(1, minDim)), 0, 1);
  return base * (1 - t * (1 - keep));
}

/** Bit names: none below 2.5×, full by 3.5×. */
export function bitLabelAlpha(zoom: number): number {
  'worklet';
  return clamp((zoom - LABEL_FROM) / (LABEL_FULL - LABEL_FROM), 0, 1);
}

/** Which dots get their names: the selected one first, then near to far, one per 160 × 24 pt cell (a name also
 *  claims the cell to its right, so two never overlap), on screen, at most LABEL_MAX. Below 2.5× nothing. Returns the
 *  indices, -1 after the last. The name is drawn at (sx + r + 4, sy). */
export function pickLabels(p: Proj, order: Uint16Array, n: number, w: number, h: number, zoom: number, selected: number, max = LABEL_MAX): Int32Array {
  'worklet';
  const out = new Int32Array(max).fill(-1);
  if (zoom < LABEL_FROM || n === 0) return out;
  const cols = Math.ceil(w / CELL_W) + 2, rows = Math.ceil(h / CELL_H) + 1;
  const used = new Uint8Array(cols * rows);
  let k = 0;
  // one loop, no inner function: the worklets plugin hands a nested 'worklet' a COPY of an outer `let`, so a helper
  // doing `out[k++]` never advanced this `k` and the loop never stopped at `max` (iOS dev review, 2026-11-02)
  for (let s = -1; s < n && k < max; s++) {
    const i = s < 0 ? selected : order[n - 1 - s];
    if (s >= 0 && i === selected) continue;
    if (i < 0 || i >= n || !(p.alpha[i] >= 0.15)) continue; // also NaN (a node without x, y or z)
    const lx = p.sx[i] + p.r[i] + 4, ly = p.sy[i];
    if (lx < 0 || lx > w - 24 || ly < 8 || ly > h - 8) continue;
    const a = Math.floor(ly / CELL_H) * cols + Math.floor(lx / CELL_W);
    if (used[a] || used[a + 1]) continue;
    used[a] = 1; used[a + 1] = 1;
    out[k++] = i;
  }
  return out;
}

/** A name cut to fit a 160 pt cell at 12 pt (about 22 characters). */
export function shortLabel(s: string, max = 22): string {
  'worklet';
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}

/** Half the width of a name's tap box: the name's width (≈ 6.8 pt a character at 12 pt semibold) plus 12, at most 70. */
export function nameHalfWidth(label: string): number {
  'worklet';
  return Math.min(70, label.length * 3.4 + 12);
}
/** A name's tap box is 44 pt tall (HIG/buttons: "a hit region of at least 44x44 pt"). */
export const NAME_HALF_H = 22;

/** Whether a group's name takes a tap: visible enough, and not the group you are already in (zoomed, near the centre),
 *  so a tap there selects the dot under it instead of flying to the same place again (iOS design review). */
export function groupNameTappable(alpha: number, zoom: number, distPx: number, minDim: number): boolean {
  'worklet';
  if (alpha < 0.4) return false;
  return !(zoom > 2 && distPx < 0.15 * minDim);
}

/** What a tap means, in this order (iOS design review): a dot right under the finger (6 pt) is selected; then a bit's
 *  name (once names are at least half shown; box = the name's width, 24 pt tall like its cell); then a group's name
 *  (`groupNameTappable`, a 44 pt tall box) flies there; then the nearest dot within 18 pt; else nothing.
 *  Returns kind 0 = a dot (idx = node), 1 = a group (idx = position in `groups`), -1 = nothing. */
export type Tap = { kind: 0 | 1 | -1; idx: number };
export function resolveTap(p: Proj, order: Uint16Array, n: number, w: number, h: number, cam: Cam, selected: number, px: number, py: number, nameW: ArrayLike<number>, groups: { cx: number; cy: number; cz: number; label: string }[]): Tap {
  'worklet';
  const direct = nearestIndex(p, n, px, py, 6);
  if (direct >= 0) return { kind: 0, idx: direct };
  const zoom = Math.exp(cam.lz);
  if (bitLabelAlpha(zoom) >= 0.5) {
    const picked = pickLabels(p, order, n, w, h, zoom, selected);
    for (let k = 0; k < picked.length && picked[k] >= 0; k++) {
      const i = picked[k];
      const lx = p.sx[i] + p.r[i] + 4;
      if (px >= lx - 4 && px <= lx + nameW[i] + 4 && Math.abs(py - p.sy[i]) <= CELL_H / 2) return { kind: 0, idx: i };
    }
  }
  const minDim = Math.min(w, h);
  for (let k = 0; k < groups.length; k++) {
    const g = groups[k];
    const q = projectPoint(g.cx, g.cy, g.cz, cam, w, h);
    const dist = Math.hypot(q.sx - w / 2, q.sy - h / 2);
    if (!groupNameTappable(groupLabelAlpha(q.depth, q.fade, zoom, dist, minDim), zoom, dist, minDim)) continue;
    if (Math.abs(px - q.sx) <= nameHalfWidth(g.label) && Math.abs(py - q.sy) <= NAME_HALF_H) return { kind: 1, idx: k };
  }
  const near = nearestIndex(p, n, px, py);
  return near >= 0 ? { kind: 0, idx: near } : { kind: -1, idx: -1 };
}

/** A short steady spin: radians per second, zero under Reduce Motion. */
export function spinStep(reduceMotion: boolean, dtMs: number, radPerS = 0.12): number {
  'worklet';
  return reduceMotion ? 0 : (radPerS * dtMs) / 1000;
}

/** How often the web re-projects: ~30 frames a second (a rAF loop re-rendering 1,600 SVG dots starves the JS thread). */
export const SPIN_FRAME_MS = 33;

/** Whether the turn should run at all: a cloud to turn, a canvas to draw on, the screen in front, his hand off it. */
export function shouldSpin(a: { spinning: boolean; reduceMotion: boolean; nodes: number; w: number; h: number; focused: boolean }): boolean {
  'worklet';
  return a.spinning && !a.reduceMotion && a.nodes > 0 && a.w > 0 && a.h > 0 && a.focused;
}

/** Momentum after a flick on the web (the phone uses Reanimated's withDecay): velocity decays by `perSecond` each second. */
export function decayStep(v: number, dtMs: number, perSecond = 0.06): number {
  'worklet';
  const nv = v * Math.pow(perSecond, dtMs / 1000);
  return Math.abs(nv) < 0.02 ? 0 : nv;
}

/** A flight on the web (the phone uses withTiming): `t` in [0, 1] eased out (cubic), every field straight across. */
export function lerpCam(a: Cam, b: Cam, t: number): Cam {
  'worklet';
  const e = 1 - Math.pow(1 - clamp(t, 0, 1), 3);
  return { yaw: a.yaw + (b.yaw - a.yaw) * e, pitch: a.pitch + (b.pitch - a.pitch) * e, tx: a.tx + (b.tx - a.tx) * e, ty: a.ty + (b.ty - a.ty) * e, tz: a.tz + (b.tz - a.tz) * e, lz: a.lz + (b.lz - a.lz) * e };
}

function r1(n: number): number {
  'worklet';
  return Math.round(n * 10) / 10;
}
/** One SVG path for many circles (the web: one element per colour × depth band keeps 1,600 dots cheap). Every number
 *  is rounded to 0.1 (147.49999999999997 would make the string the native parser reads half again as long). */
export function circlesPath(p: Proj, idx: ArrayLike<number>, grow = 0): string {
  'worklet';
  let s = '';
  for (let j = 0; j < idx.length; j++) {
    const i = idx[j];
    const r = r1(p.r[i] + grow), x = r1(p.sx[i]), y = r1(p.sy[i]);
    const dia = r1(r * 2);
    s += `M${r1(x - r)} ${y}a${r} ${r} 0 1 0 ${dia} 0a${r} ${r} 0 1 0 -${dia} 0`;
  }
  return s;
}

/** Three alpha bands for the web's SVG batching: the band's alpha is the middle of its range. */
export function band(alpha: number): 0 | 1 | 2 {
  'worklet';
  return alpha < 0.5 ? 0 : alpha < 0.8 ? 1 : 2;
}
export const BAND_ALPHA = [0.4, 0.68, 0.95] as const;
