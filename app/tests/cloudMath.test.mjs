// Unit test for rules/cloudMath.ts (the cloud's orbit camera, perspective, gestures' moves, hit test, names, batching).
// node --experimental-strip-types tests/cloudMath.test.mjs   (npm run test:unit)
import { readFileSync } from "node:fs";
import * as M from "../src/rules/cloudMath.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const near = (a, b, eps = 1e-4) => Math.abs(a - b) < eps;
const W = 300, H = 300, R = M.fitRadius(W, H);

// --- every function carries 'worklet' (the build-16 crash class; wrapAngle lacked it in builds 17-18) ---
const src = readFileSync(new URL("../src/rules/cloudMath.ts", import.meta.url), "utf8");
const fnRx = /function\s+(\w+)\s*\(.*\{[ \t]*\n\s*(['"]worklet['"])?/g;
const missing = [];
let fm, fnCount = 0;
while ((fm = fnRx.exec(src))) { fnCount++; if (!fm[2]) missing.push(fm[1]); }
check(fnCount >= 20 && missing.length === 0, `every function in cloudMath carries 'worklet' (${fnCount} found${missing.length ? `; missing: ${missing.join(", ")}` : ""})`);
// no nested worklet: the plugin hands a nested 'worklet' a COPY of the outer `let`s, so `out[k++]` in a helper never
// advanced the caller's k (pickLabels never stopped at 30; iOS dev review 2026-11-02). Node runs without the plugin,
// so only this structural check can see it.
const code = src.replace(/\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, "");
check(!/=>\s*\{\s*['"]worklet['"]/.test(code) && (code.match(/['"]worklet['"]/g) || []).length === fnCount, "no nested 'worklet' function in cloudMath (one directive per top-level function)");
// the phone file calls only cloudMath functions (or worklets of its own) from worklets: no other module's function
const phone = readFileSync(new URL("../src/lib/Cloud.native.tsx", import.meta.url), "utf8");
const calls = [...phone.matchAll(/\bM\.(\w+)\(/g)].map((x) => x[1]);
const exported = new Set(Object.entries(M).filter(([, v]) => typeof v === "function").map(([k]) => k));
const unknown = [...new Set(calls)].filter((c) => !exported.has(c));
check(calls.length > 5 && unknown.length === 0, `Cloud.native calls only cloudMath worklets through M. (${unknown.join(", ") || "all known"})`);
// every function the phone file defines and calls from a worklet carries 'worklet' (named function declarations)
const phoneFns = [...phone.matchAll(/function\s+(\w+)\s*\(.*\{[ \t]*\n\s*(['"]worklet['"])?/g)];
const jsOnly = new Set(["flatten", "Cloud", "ClusterLabel", "makePaints"]);
const phoneMissing = phoneFns.filter((x) => !x[2] && !jsOnly.has(x[1])).map((x) => x[1]);
check(phoneMissing.length === 0, `Cloud.native's own helpers carry 'worklet' unless listed as JS-only (${phoneMissing.join(", ") || "ok"})`);
// the self-test: the regex must catch a planted offender, else it proves nothing
const planted = "export function bad(a: number): number {\n  return a;\n}\nexport function good(a: number): number {\n  'worklet';\n  return a;\n}";
const pm = [...planted.matchAll(/function\s+(\w+)\s*\(.*\{[ \t]*\n\s*(['"]worklet['"])?/g)].map((x) => [x[1], !!x[2]]);
check(pm.length === 2 && pm[0][1] === false && pm[1][1] === true, "the directive check catches a planted function without 'worklet'");

// --- the camera at home reproduces today's fit ---
let p = M.projectPoint(0, 0, 0, M.HOME, W, H);
check(near(p.sx, 150) && near(p.sy, 150) && near(p.depth, 0.5) && near(p.fade, 1), "home: the world's centre is the canvas centre, mid depth, drawn");
p = M.projectPoint(1, 0, 0, M.HOME, W, H);
check(near(p.sx, 150 + R) && near(p.sy, 150), "home: x = 1 lands at the fit radius (as before: pers 1 at mid depth)");
p = M.projectPoint(1, 1, 0, M.HOME, W, H);
check(p.sx > 150 && p.sy < 150, "+x is right, +y is up");
const nearP = M.projectPoint(0.5, 0, 1, M.HOME, W, H), farP = M.projectPoint(0.5, 0, -1, M.HOME, W, H);
check(nearP.depth === 1 && farP.depth === 0 && nearP.grow > 1 && farP.grow < 1, "z = 1 is nearest (depth 1, grows), z = -1 the far edge");
check(nearP.sx - 150 > (farP.sx - 150) * 1.4 && nearP.sx - 150 < (farP.sx - 150) * 1.8, "perspective at zoom 1 is mild (near spreads ~1.67× the far)");

// --- orbit directions and the pitch clamp ---
let cam = M.orbitBy(M.HOME, 30, 0, W);
p = M.projectPoint(0, 0, 1, cam, W, H);
check(p.sx > 150, "drag right: the front of the cloud moves right");
cam = M.orbitBy(M.HOME, 0, 30, W);
p = M.projectPoint(0, 0, 1, cam, W, H);
const top = M.projectPoint(0, 1, 0, cam, W, H);
check(p.sy > 150 && top.depth > 0.5, "drag down: the front moves down and the top tilts toward you");
check(near(M.orbitBy(M.HOME, W, 0, W).yaw, -Math.PI), "a drag across the canvas width turns 180°");
check(M.orbitBy(M.HOME, 0, 10000, W).pitch === M.PITCH_LIMIT && M.orbitBy(M.HOME, 0, -10000, W).pitch === -M.PITCH_LIMIT, "pitch is clamped (never over the top)");

// --- zoom bounds, zoom about a point, pan ---
check(M.clampLz(10) === M.LZ_MAX && M.clampLz(-10) === M.LZ_MIN && near(Math.exp(M.LZ_MAX), 12) && near(Math.exp(M.LZ_MIN), 0.6), "zoom is bounded 0.6× to 12×");
let worst = 0;
let seed = 7;
const rnd = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647; };
for (let t = 0; t < 200; t++) {
  const c0 = { yaw: rnd() * 6.28, pitch: (rnd() - 0.5) * 2, tx: (rnd() - 0.5) * 0.6, ty: (rnd() - 0.5) * 0.6, tz: (rnd() - 0.5) * 0.6, lz: (rnd() - 0.5) * 2 };
  const ox = (rnd() - 0.5) * 120, oy = (rnd() - 0.5) * 120;
  // the world point on the target plane under (ox, oy): P = t + right·(ox/RZ) + up·(-oy/RZ)
  const Z0 = Math.exp(c0.lz), cy = Math.cos(c0.yaw), sy = Math.sin(c0.yaw), cp = Math.cos(c0.pitch), sp = Math.sin(c0.pitch);
  const r = [cy, 0, -sy], u = [-sp * sy, cp, -sp * cy];
  const P = [0, 1, 2].map((a) => [c0.tx, c0.ty, c0.tz][a] + r[a] * ox / (R * Z0) + u[a] * (-oy) / (R * Z0));
  const before = M.projectPoint(P[0], P[1], P[2], c0, W, H);
  const c1 = M.zoomAbout(c0, c0.lz + (rnd() - 0.5) * 1.5, ox, oy, R);
  const after = M.projectPoint(P[0], P[1], P[2], c1, W, H);
  worst = Math.max(worst, Math.abs(before.sx - (150 + ox)) + Math.abs(before.sy - (150 + oy)), Math.abs(after.sx - before.sx) + Math.abs(after.sy - before.sy));
}
check(worst < 1e-3, `zoomAbout keeps the point under the fingers (200 random cameras, worst drift ${worst.toExponential(1)} pt)`);
cam = { yaw: 0.7, pitch: 0.3, tx: 0.1, ty: -0.1, tz: 0, lz: 0.5 };
const P0 = M.projectPoint(cam.tx, cam.ty, cam.tz, cam, W, H);
const moved = M.panBy(cam, 25, -10, R);
const P1 = M.projectPoint(cam.tx, cam.ty, cam.tz, moved, W, H);
check(near(P1.sx - P0.sx, 25, 1e-3) && near(P1.sy - P0.sy, -10, 1e-3), "panBy: the target plane follows the fingers");
const far = M.panBy(M.HOME, -1e6, 1e6, R);
check(Math.abs(far.tx) <= M.TARGET_LIMIT && Math.abs(far.ty) <= M.TARGET_LIMIT && Math.abs(far.tz) <= M.TARGET_LIMIT, "the target stays inside the cloud's box (part of it is always on screen)");
const dt = M.doubleTapTarget(M.HOME, 40, -20, R);
check(near(dt.lz, Math.LN2) && !M.isHome(dt), "double tap: 2× in about the point");
check(M.isHome(M.doubleTapTarget({ ...M.HOME, yaw: 1, lz: M.LZ_MAX, tx: 0.3 }, 0, 0, R)), "double tap at 12× goes to the whole map");
check(M.isHome(M.homeFrom({ yaw: 2, pitch: 0.5, tx: 0.2, ty: 0, tz: 0, lz: 1 })) && M.homeFrom({ ...M.HOME, yaw: 2 }).yaw === 2, "Whole map: upright, centred, zoom 1, the yaw kept");

// --- the near fade and dot growth ---
const xs = new Float32Array([0, 0, 0, 0.5]), ys = new Float32Array([0, 0, 0, 0]), zs = new Float32Array([0, 1, -1, 0]);
const pr = M.newProj(4);
M.projectInto(4, xs, ys, zs, M.HOME, W, H, pr);
check([0, 1, 2, 3].every((i) => pr.alpha[i] > 0), "zoom 1: nothing is culled");
check(near(pr.sx[3], M.projectPoint(0.5, 0, 0, M.HOME, W, H).sx) && near(pr.r[1], M.dotRadius(1, M.EYE / 3)), "projectInto agrees with projectPoint");
M.projectInto(4, xs, ys, zs, { ...M.HOME, lz: M.LZ_MAX }, W, H, pr);
check(pr.alpha[1] === 0 && pr.alpha[0] > 0 && pr.alpha[2] > 0, "12×: the dot in front of the camera is not drawn, the target and behind are");
check(M.nearestIndex(pr, 4, pr.sx[1], pr.sy[1]) !== 1, "a dot that is not drawn is never hit");
const g1 = M.projectPoint(0.05, 0, 0, M.HOME, W, H), g8 = M.projectPoint(0.05, 0, 0, { ...M.HOME, lz: Math.log(8) }, W, H);
const gap1 = g1.sx - 150, gap8 = g8.sx - 150;
check(near(gap8 / gap1, 8, 1e-3) && M.dotRadius(0.5, g8.grow) / M.dotRadius(0.5, g1.grow) < 3, "8×: gaps grow 8×, dots under 3× (a dense spot separates)");

// --- hit test, depth order ---
const nodes = [
  { id: "a", short_name: "A", nights: 0, x: 0, y: 0, z: 1, cluster: 0 },
  { id: "b", short_name: "B", nights: 0, x: 0, y: 0, z: -1, cluster: 1 },
  { id: "c", short_name: "C", nights: 0, x: 1, y: 1, z: 0, cluster: 0 },
];
const nx = new Float32Array(nodes.map((d) => d.x)), ny = new Float32Array(nodes.map((d) => d.y)), nz = new Float32Array(nodes.map((d) => d.z));
const pj = M.newProj(3);
M.projectInto(3, nx, ny, nz, M.HOME, W, H, pj);
const ord = M.depthOrder(pj.depth, 3);
check(ord[0] === 1 && ord[2] === 0, "far dots first, near dots last (they draw on top)");
check(pj.r[0] > pj.r[1] && pj.alpha[0] > pj.alpha[1] && near(pj.alpha[0], 1) && near(pj.alpha[1], M.ALPHA_MIN), "near = bigger and brighter");
check(M.nearestIndex(pj, 3, 150, 150) === 0, "a tap at the centre hits the near dot there, not the one behind");
check(M.nearestIndex(pj, 3, 20, 280) === -1, "a tap in empty space hits nothing");
check(M.nearestIndex(pj, 3, pj.sx[2] + pj.r[2] + 10, pj.sy[2]) === 2, "slop reaches a dot just past its edge");
const zoomedCam = M.zoomAbout(M.HOME, Math.log(4), pj.sx[2] - 150, pj.sy[2] - 150, R);
M.projectInto(3, nx, ny, nz, zoomedCam, W, H, pj);
check(M.nearestIndex(pj, 3, pj.sx[2], pj.sy[2]) === 2, "zoomed in: the tap hits against the current projection (no view.scale)");

// --- groups: spread, fly, label fades ---
const clusters = [{ id: 0, label: "Zero", size: 2, cx: 0.5, cy: 0.5, cz: 0.5 }, { id: 1, label: "One", size: 1, cx: 0, cy: 0, cz: -1 }];
const sp = M.spreads(nodes, clusters);
check(sp.length === 2 && near(sp[1], 0) === true && sp[0] > 0.5, "spreads: RMS distance from each group's centre");
const fl = M.flyTo({ ...M.HOME, yaw: 1, pitch: 0.2 }, clusters[0], sp[0]);
check(fl.tx === 0.5 && fl.yaw === 1 && fl.pitch === 0.2 && Math.exp(fl.lz) >= 1.6 && Math.exp(fl.lz) <= 6, "flyTo: the target is the group's centre, zoom 1.6-6×, yaw and pitch kept");
check(near(Math.exp(M.flyTo(M.HOME, clusters[1], 0).lz), 6), "a tight group flies in to 6×");
p = M.projectPoint(0.5, 0.5, 0.5, fl, W, H);
check(near(p.sx, 150) && near(p.sy, 150), "after flying, the group's centre is the canvas centre");
check(near(M.groupLabelAlpha(1, 1, 1, 200, 300), 1) && near(M.groupLabelAlpha(1, 1, 4, 200, 300), 0) && near(M.groupLabelAlpha(1, 1, 4, 0, 300), 1), "group names: all at 1×; zoomed, only the one near the centre");
check(M.bitLabelAlpha(2) === 0 && near(M.bitLabelAlpha(3), 0.5) && M.bitLabelAlpha(4) === 1, "bit names: none under 2.5×, full by 3.5×");

// --- which dots get names ---
const many = 400, mx = new Float32Array(many), my = new Float32Array(many), mz = new Float32Array(many);
for (let i = 0; i < many; i++) { mx[i] = (rnd() - 0.5) * 0.4; my[i] = (rnd() - 0.5) * 0.4; mz[i] = (rnd() - 0.5) * 0.4; }
const mp = M.newProj(many);
const zc = { ...M.HOME, lz: Math.log(4) };
M.projectInto(many, mx, my, mz, zc, W, H, mp);
const mo = M.depthOrder(mp.depth, many);
check(M.pickLabels(mp, mo, many, W, H, 2, -1)[0] === -1, "under 2.5× no names");
const picked = [...M.pickLabels(mp, mo, many, W, H, 4, 17)].filter((i) => i >= 0);
check(picked.length > 0 && picked.length <= M.LABEL_MAX && picked[0] === 17, `4×: names for ${picked.length} dots, at most ${M.LABEL_MAX}, the selected one first`);
const cells = picked.map((i) => [Math.floor((mp.sx[i] + mp.r[i] + 4) / M.CELL_W), Math.floor(mp.sy[i] / M.CELL_H)]);
let overlap = false;
for (let a = 0; a < cells.length; a++) for (let b = a + 1; b < cells.length; b++) if (cells[a][1] === cells[b][1] && Math.abs(cells[a][0] - cells[b][0]) <= 1) overlap = true;
check(!overlap, "no two names share a row within one cell of each other");
check(picked.every((i) => mp.sx[i] + mp.r[i] + 4 <= W - 24 && mp.sy[i] >= 8 && mp.sy[i] <= H - 8), "every name starts on screen");
const all = [...M.pickLabels(mp, mo, many, W, H, 12, -1, 5)];
check(all.filter((i) => i >= 0).length === 5 && all.length === 5, "pickLabels stops at its cap");

// --- what a tap means: a dot under the finger, a bit's name, a group's name, the nearest dot ---
const tx = new Float32Array([0, 0.1, -0.5]), ty = new Float32Array([0, 0.1, 0.4]), tz = new Float32Array([0, 0, 0]);
const tp = M.newProj(3);
M.projectInto(3, tx, ty, tz, M.HOME, W, H, tp);
const tord = M.depthOrder(tp.depth, 3);
const grp = [{ cx: 0, cy: 0, cz: 0, label: "Raccoons" }];
const widths = new Float32Array([60, 60, 60]);
check(M.resolveTap(tp, tord, 3, W, H, M.HOME, -1, tp.sx[0], tp.sy[0], widths, grp).kind === 0, "a tap right on a dot selects it, even under a group's name");
check(JSON.stringify(M.resolveTap(tp, tord, 3, W, H, M.HOME, -1, tp.sx[0] + 30, tp.sy[0] + 15, widths, grp)) === '{"kind":1,"idx":0}', "a tap on a group's name (off any dot) flies to the group; the box is 44 pt tall");
check(M.resolveTap(tp, tord, 3, W, H, M.HOME, -1, tp.sx[0] + 30, tp.sy[0] + 30, widths, grp).kind !== 1, "below the 44 pt box it is not the group's name");
check(M.resolveTap(tp, tord, 3, W, H, M.HOME, -1, 5, 295, widths, grp).kind === -1, "a tap on nothing clears");
const inGroup = { ...M.HOME, lz: Math.log(4) };
M.projectInto(3, tx, ty, tz, inGroup, W, H, tp);
check(M.resolveTap(tp, M.depthOrder(tp.depth, 3), 3, W, H, inGroup, -1, 150 + 30, 150 + 15, widths, grp).kind !== 1, "zoomed into a group, its own name at the centre no longer takes the tap");
const nm = M.pickLabels(tp, M.depthOrder(tp.depth, 3), 3, W, H, 4, -1).filter((i) => i >= 0);
const li = nm.find((i) => i === 1);
check(li === 1 && JSON.stringify(M.resolveTap(tp, M.depthOrder(tp.depth, 3), 3, W, H, inGroup, -1, tp.sx[1] + tp.r[1] + 4 + 50, tp.sy[1], widths, grp)) === '{"kind":0,"idx":1}', "zoomed in, a tap on a bit's name selects that bit");
check(M.resolveTap(tp, M.depthOrder(tp.depth, 3), 3, W, H, inGroup, -1, tp.sx[1] + tp.r[1] + 4 + 90, tp.sy[1], widths, grp).kind !== 0 || false, "past the end of the name it is not the name");
check(M.nameHalfWidth("x".repeat(40)) === 70 && M.nameHalfWidth("Raccoons") > 30, "a group name's tap box follows the name, at most 140 pt wide");
check(M.groupNameTappable(1, 1, 0, 300) && !M.groupNameTappable(0.3, 1, 200, 300) && !M.groupNameTappable(1, 3, 10, 300), "group names: too faint or already there = not tappable");

check(M.shortLabel("a".repeat(30)).length === 22 && M.shortLabel("short") === "short", "names are cut to 22 characters with an ellipsis");

// --- the web's batching, the spin, momentum ---
const path = M.circlesPath(pj, [0, 1, 2]);
check(path.split("M").length - 1 === 3 && !/\d\.\d{2,}/.test(path), "one path, one circle per dot, every number rounded to one decimal");
check(path.split("a").length - 1 === 6, "two arcs per circle");
check(M.band(0.3) === 0 && M.band(0.6) === 1 && M.band(0.9) === 2 && M.BAND_ALPHA.length === 3, "three alpha bands for the web");
check(M.spinStep(true, 1000) === 0 && near(M.spinStep(false, 1000), 0.12), "the spin stops under Reduce Motion");
check(M.SPIN_FRAME_MS >= 30 && M.SPIN_FRAME_MS <= 40, `the web re-projects at ~30 fps (${M.SPIN_FRAME_MS} ms), not every frame`);
check(near(M.wrapAngle(2 * Math.PI + 0.5), 0.5) && near(M.wrapAngle(-0.5), 2 * Math.PI - 0.5) && M.wrapAngle(1) === 1, "the angle wraps into [0, 2π)");
const la = { ...M.HOME, yaw: 1 }, lb = { yaw: 2, pitch: 0.5, tx: 0.2, ty: 0, tz: -0.2, lz: 1 };
check(M.isHome(M.lerpCam(M.HOME, lb, 0)) && near(M.lerpCam(la, lb, 1).lz, 1) && M.lerpCam(la, lb, 0.5).lz > 0.5, "web flights ease out from a to b");
check(M.decayStep(1, 1000) < 0.1 && M.decayStep(1, 16) > 0.9 && M.decayStep(0.01, 16) === 0, "web momentum decays and stops");
const base = { spinning: true, reduceMotion: false, nodes: 10, w: 300, h: 300, focused: true };
check(M.shouldSpin(base) === true, "the turn runs when there is a cloud, a canvas and the screen is in front");
check(M.shouldSpin({ ...base, nodes: 0 }) === false, "nothing to turn before the cloud arrives");
check(M.shouldSpin({ ...base, w: 0 }) === false && M.shouldSpin({ ...base, h: 0 }) === false, "no canvas yet: no turn");
check(M.shouldSpin({ ...base, focused: false }) === false, "a pushed bit page or another tab stops the turn");
check(M.shouldSpin({ ...base, reduceMotion: true }) === false && M.shouldSpin({ ...base, spinning: false }) === false, "Reduce Motion and his own Still both stop it");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
