// The cloud's geometry (lib/Cloud.tsx; api-dev/cloud_api.py gives x, y, z in [-1, 1]): a turn about the vertical axis,
// depth into size and fade, a projection onto the canvas, and the nearest dot to a tap. Pure: tests/cloudMath.test.mjs.
export type CloudNode = { id: string; short_name: string; title?: string; stage?: string | null; nights: number; heat?: number | null; touched?: string | null; x: number; y: number; z: number; cluster: number };
export type CloudCluster = { id: number; label: string; size: number; cx: number; cy: number; cz: number };
export type CloudMap = { rev?: string | null; nodes: CloudNode[]; clusters: CloudCluster[]; counts: { jokes: number; placed: number; with_vectors: number; k?: number } };
export type Dot = { id: string; sx: number; sy: number; depth: number; r: number; alpha: number; cluster: number };

export const R_MIN = 2.6;
export const R_MAX = 8.5;
export const ALPHA_MIN = 0.3;

/** Turn (x, z) about the y axis by `theta` radians; depth in [0, 1], 1 = nearest the eye. */
export function turn(x: number, z: number, theta: number): { x: number; z: number; depth: number } {
  const c = Math.cos(theta), s = Math.sin(theta);
  const xr = x * c - z * s;
  const zr = x * s + z * c;
  return { x: xr, z: zr, depth: Math.max(0, Math.min(1, (zr + 1) / 2)) };
}

/** The dots on a canvas of w × h, world [-1, 1] mapped into the padded square, far dots first (they draw underneath). */
export function projectAll(nodes: CloudNode[], w: number, h: number, theta: number, pad = 28): Dot[] {
  const R = Math.max(1, (Math.min(w, h) - pad * 2) / 2);
  const cx = w / 2, cy = h / 2;
  const out: Dot[] = nodes.map((n) => {
    const t = turn(n.x, n.z, theta);
    const pers = 0.82 + 0.36 * t.depth; // a little perspective: near things spread, far things gather
    return { id: n.id, sx: cx + t.x * R * pers, sy: cy - n.y * R * pers, depth: t.depth, r: R_MIN + (R_MAX - R_MIN) * t.depth, alpha: ALPHA_MIN + (1 - ALPHA_MIN) * t.depth, cluster: n.cluster };
  });
  out.sort((a, b) => a.depth - b.depth);
  return out;
}

/** The dot under a tap, or null: nearest centre within `slop` world pixels (bigger slop for the small far dots too). */
export function nearest(dots: Dot[], px: number, py: number, slop = 18): Dot | null {
  let best: Dot | null = null, bd = Infinity;
  for (const d of dots) {
    const dx = d.sx - px, dy = d.sy - py;
    const dist = Math.sqrt(dx * dx + dy * dy) - d.r;
    if (dist < bd && dist <= slop) { bd = dist; best = d; }
  }
  return best;
}

const r1 = (n: number) => Math.round(n * 10) / 10;
/** One SVG path for many circles (one element per colour × depth band keeps 1,600 dots cheap). Every number is
 *  rounded to 0.1 (a difference like 150.1 - 2.6 otherwise prints as 147.49999999999997: the string the native parser
 *  reads each frame would be half again as long). */
export function circlesPath(dots: Dot[]): string {
  let s = '';
  for (const d of dots) {
    const r = r1(d.r), x = r1(d.sx), y = r1(d.sy);
    const dia = r1(r * 2);
    s += `M${r1(x - r)} ${y}a${r} ${r} 0 1 0 ${dia} 0a${r} ${r} 0 1 0 -${dia} 0`;
  }
  return s;
}

/** Three depth bands (far · mid · near): the band's alpha is the middle of its range. */
export function band(depth: number): 0 | 1 | 2 {
  return depth < 1 / 3 ? 0 : depth < 2 / 3 ? 1 : 2;
}
export const BAND_ALPHA = [0.35, 0.65, 1] as const;

/** The angle that brings a cluster's centre to the front (depth 1): turn so its (x, z) points at +z. */
export function faceAngle(cx: number, cz: number): number {
  if (Math.abs(cx) < 1e-6 && Math.abs(cz) < 1e-6) return 0;
  return Math.atan2(cx, cz); // after turn(theta): z' = x·sin θ + z·cos θ, largest when θ = atan2(x, z)
}

/** A short steady spin: radians per second, zero under Reduce Motion. */
export function spinStep(reduceMotion: boolean, dtMs: number, radPerS = 0.12): number {
  return reduceMotion ? 0 : (radPerS * dtMs) / 1000;
}

/** How often the turn re-projects: ~30 frames a second is smooth for a 0.12 rad/s drift and half the work of 60. */
export const SPIN_FRAME_MS = 33;

/** Keep the angle in [0, 2π) so a long spin never grows the float (and a memo on theta stays meaningful). */
export function wrapAngle(theta: number): number {
  const t = theta % (2 * Math.PI);
  return t < 0 ? t + 2 * Math.PI : t;
}

/** Whether the turn should run at all: a cloud to turn, a canvas to draw on, the screen in front, his hand off it. */
export function shouldSpin(a: { spinning: boolean; reduceMotion: boolean; nodes: number; w: number; h: number; focused: boolean }): boolean {
  return a.spinning && !a.reduceMotion && a.nodes > 0 && a.w > 0 && a.h > 0 && a.focused;
}
