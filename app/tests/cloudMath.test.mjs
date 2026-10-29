// Unit test for lib/cloudMath.ts (the cloud's turn, depth, projection, hit test, batching).
// node --experimental-strip-types tests/cloudMath.test.mjs   (npm run test:unit)
import * as M from "../src/lib/cloudMath.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const near = (a, b, eps = 1e-6) => Math.abs(a - b) < eps;

let t = M.turn(1, 0, 0);
check(near(t.x, 1) && near(t.z, 0) && near(t.depth, 0.5), "no turn: unchanged, mid depth");
t = M.turn(1, 0, Math.PI / 2);
check(near(t.x, 0) && near(t.z, 1) && near(t.depth, 1), "a quarter turn brings +x to the front");
t = M.turn(0, -1, 0);
check(near(t.depth, 0), "z = -1 is the far edge");
const f = M.turn(0.3, 0.7, M.faceAngle(0.3, 0.7));
check(near(f.x, 0) && near(f.z, Math.hypot(0.3, 0.7)), "faceAngle turns a cluster centre to the front (x to 0, z to its full reach)");
check(M.faceAngle(0, 0) === 0, "a centre at the origin needs no turn");

const nodes = [
  { id: "a", short_name: "A", nights: 0, x: 0, y: 0, z: 1, cluster: 0 },
  { id: "b", short_name: "B", nights: 0, x: 0, y: 0, z: -1, cluster: 1 },
  { id: "c", short_name: "C", nights: 0, x: 1, y: 1, z: 0, cluster: 0 },
];
const dots = M.projectAll(nodes, 300, 300, 0);
check(dots[0].id === "b" && dots[dots.length - 1].id === "a", "far dots first, near dots last (they draw on top)");
const a = dots.find((d) => d.id === "a"), b = dots.find((d) => d.id === "b"), c = dots.find((d) => d.id === "c");
check(a.r > b.r && a.alpha > b.alpha && near(a.alpha, 1) && near(b.alpha, M.ALPHA_MIN), "near = bigger and brighter");
check(near(a.sx, 150) && near(a.sy, 150), "the centre of the world is the centre of the canvas");
check(c.sx > 150 && c.sy < 150, "+x is right, +y is up");
check(M.nearest(dots, 150, 150).id === "a", "a tap at the centre hits the near dot there");
check(M.nearest(dots, 20, 280) === null, "a tap in empty space hits nothing");
check(M.nearest(dots, c.sx + 10, c.sy).id === "c", "slop reaches a dot just past its edge");
const path = M.circlesPath(dots);
check(path.split("M").length - 1 === 3 && path.includes("a"), "one path, one circle per dot");
check(M.band(0.1) === 0 && M.band(0.5) === 1 && M.band(0.9) === 2 && M.BAND_ALPHA[2] === 1, "three depth bands");
check(M.spinStep(true, 1000) === 0 && near(M.spinStep(false, 1000), 0.12), "the spin stops under Reduce Motion");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
