// Bits › Map › the cloud on the web: every bit placed by what it SAYS (GET /api/map/cloud), seen through the same orbit
// camera as the phone (rules/cloudMath; spec docs/design/SPEC-2026-11-02-cloud-3d.md), drawn as SVG paths, one per
// group × alpha band. A pointer drag orbits (momentum on release), the wheel or a trackpad pinch zooms about the
// cursor, two touch pointers pinch and pan, a double tap zooms in. The camera lives in a ref; the SVG re-renders at
// most every ~33 ms (a rAF loop re-rendering 1,600 dots starves the JS thread: taps stop landing, HANDOVER traps).
// The phone draws the same cloud with Skia (lib/Cloud.native). The chrome (states, card, chips, zoom) is lib/CloudChrome.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import { router } from 'expo-router';
import Svg, { Path, Text as SvgText } from 'react-native-svg';
import { useApi } from './api';
import { CloudFrame, useInFront } from './CloudChrome';
import { haptics } from './haptics';
import { slotColor } from './Galaxy';
import * as M from '@/rules/cloudMath';
import { C, F } from './theme';
import { useReduceMotion } from './ui';

const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
const FLIGHT_MS = 350;

type Ptr = { x: number; y: number };

export function Cloud({ onLinks }: { onLinks: () => void }) {
  const reduce = useReduceMotion();
  const inFront = useInFront();
  const m = useApi<M.CloudMap>('/api/map/cloud?cap=1600', { poll: 0 });
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [spinning, setSpinning] = useState(!reduce);
  const [focus, setFocus] = useState<string | null>(null);
  const [moved, setMoved] = useState(false);
  const [motion, setMotion] = useState(false);
  const [shown, setShown] = useState<M.Cam>(M.HOME); // what the SVG draws: a snapshot of the camera ref
  const cam = useRef<M.Cam>(M.HOME);
  const vel = useRef({ yaw: 0, pitch: 0 });
  const flight = useRef<{ from: M.Cam; to: M.Cam; t0: number } | null>(null);
  const W = size.w, H = size.h;
  const nodes = useMemo(() => m.data?.nodes || [], [m.data]);
  const clusters = useMemo(() => m.data?.clusters || [], [m.data]);
  const spread = useMemo(() => M.spreads(nodes, clusters), [nodes, clusters]);
  const flat = useMemo(() => ({ x: new Float32Array(nodes.map((d) => d.x)), y: new Float32Array(nodes.map((d) => d.y)), z: new Float32Array(nodes.map((d) => d.z)) }), [nodes]);
  const byId = useMemo(() => new Map(nodes.map((d, i) => [d.id, i])), [nodes]);
  const turning = M.shouldSpin({ spinning, reduceMotion: reduce, nodes: nodes.length, w: W, h: H, focused: inFront });

  // a re-render at most every SPIN_FRAME_MS, however fast pointer events arrive
  const pending = useRef(false);
  const lastDraw = useRef(0);
  const redraw = useCallback(() => {
    if (pending.current) return;
    pending.current = true;
    setTimeout(() => { pending.current = false; lastDraw.current = now(); setShown(cam.current); }, Math.max(0, M.SPIN_FRAME_MS - (now() - lastDraw.current)));
  }, []);
  const [limit, setLimit] = useState<'max' | 'min' | null>(null);
  const settle = useCallback((c: M.Cam) => { setMoved(!M.isHome(c)); setLimit(c.lz >= M.LZ_MAX - 0.01 ? 'max' : c.lz <= M.LZ_MIN + 0.01 ? 'min' : null); }, []);
  const stopAll = useCallback(() => { vel.current = { yaw: 0, pitch: 0 }; flight.current = null; setSpinning(false); }, []);
  const goTo = useCallback((to: M.Cam) => {
    if (reduce) { cam.current = to; flight.current = null; redraw(); settle(to); return; }
    flight.current = { from: cam.current, to, t0: now() };
    settle(to);
    setMotion(true);
  }, [reduce, redraw, settle]);

  // the turn, momentum and flights: one ~30 fps timer while something moves; nothing runs when the cloud is still
  useEffect(() => {
    if ((!turning && !motion) || !inFront) return;
    let last = now();
    const t = setInterval(() => {
      const n = now(), dt = n - last;
      last = n;
      let c = cam.current, busy = false;
      const f = flight.current;
      if (f) {
        const k = (n - f.t0) / FLIGHT_MS;
        c = M.lerpCam(f.from, f.to, k);
        if (k >= 1) flight.current = null; else busy = true;
      } else if (vel.current.yaw || vel.current.pitch) {
        c = { ...c, yaw: c.yaw + (vel.current.yaw * dt) / 1000, pitch: M.clampPitch(c.pitch + (vel.current.pitch * dt) / 1000) };
        vel.current = { yaw: M.decayStep(vel.current.yaw, dt), pitch: M.decayStep(vel.current.pitch, dt) };
        busy = !!(vel.current.yaw || vel.current.pitch);
      } else if (turning) c = { ...c, yaw: M.wrapAngle(c.yaw + M.spinStep(false, dt)) };
      cam.current = c;
      setShown(c);
      if (!busy) setMotion(false);
    }, M.SPIN_FRAME_MS);
    return () => clearInterval(t);
  }, [turning, motion, inFront]);

  const view = useMemo(() => {
    const n = nodes.length;
    if (!W || !H || !n) return null;
    const pj = M.newProj(n);
    M.projectInto(n, flat.x, flat.y, flat.z, shown, W, H, pj);
    const order = M.depthOrder(pj.depth, n);
    const zoom = Math.exp(shown.lz);
    const fi = focus ? (byId.get(focus) ?? -1) : -1;
    const groups = new Map<string, { color: string; alpha: number; idx: number[] }>();
    for (let k = 0; k < n; k++) {
      const i = order[k];
      if (pj.alpha[i] <= 0) continue;
      const b = M.band(pj.alpha[i]);
      const key = `${nodes[i].cluster}:${b}`;
      let g = groups.get(key);
      if (!g) { g = { color: slotColor(nodes[i].cluster % 8), alpha: M.BAND_ALPHA[b], idx: [] }; groups.set(key, g); }
      g.idx.push(i);
    }
    const paths = [...groups.entries()].sort((a, b) => a[1].alpha - b[1].alpha).map(([key, g]) => ({ key, color: g.color, alpha: g.alpha, path: M.circlesPath(pj, g.idx) }));
    const minDim = Math.min(W, H);
    const labels = clusters.map((c) => {
      const p = M.projectPoint(c.cx, c.cy, c.cz, shown, W, H);
      return { id: c.id, label: c.label, x: p.sx, y: p.sy, alpha: M.groupLabelAlpha(p.depth, p.fade, zoom, Math.hypot(p.sx - W / 2, p.sy - H / 2), minDim) };
    });
    const la = M.bitLabelAlpha(zoom);
    const names = la > 0 ? [...M.pickLabels(pj, order, n, W, H, zoom, fi)].filter((i) => i >= 0).map((i) => ({ i, x: pj.sx[i] + pj.r[i] + 4, y: pj.sy[i] + 4, alpha: la * pj.fade[i], text: M.shortLabel(nodes[i].short_name || nodes[i].title || '') })) : [];
    const ring = fi >= 0 && pj.alpha[fi] > 0 ? M.circlesPath(pj, [fi], 5) : null;
    return { pj, order, paths, labels, names, ring };
  }, [shown, nodes, clusters, flat, W, H, focus, byId]);

  // a tap (M.resolveTap, the phone's order): a dot under the finger, a bit's name, a group's name, the nearest dot
  const nameW = useMemo(() => Float32Array.from(nodes, (d) => M.shortLabel(d.short_name || d.title || '').length * 6.8), [nodes]);
  const tapAt = useCallback((px: number, py: number) => {
    if (!view) return;
    const t = M.resolveTap(view.pj, view.order, nodes.length, W, H, shown, focus ? (byId.get(focus) ?? -1) : -1, px, py, nameW, clusters);
    if (t.kind === 1) return goTo(M.flyTo(cam.current, clusters[t.idx], spread[t.idx] ?? 0.3));
    if (t.kind !== 0) return setFocus(null);
    haptics.selection();
    const id = nodes[t.idx].id;
    if (id === focus) return router.push(`/joke/${encodeURIComponent(id)}` as any);
    setFocus(id);
  }, [view, shown, W, H, byId, nameW, clusters, spread, nodes, focus, goTo]);

  // pointers: one orbits, two pinch and pan; the wheel zooms about the cursor (a trackpad pinch is ctrl + wheel)
  const ptrs = useRef(new Map<number, Ptr>());
  const drag = useRef<{ x: number; y: number; t: number; moved: boolean; vx: number; vy: number; lastTap: number; tapX: number; tapY: number }>({ x: 0, y: 0, t: 0, moved: false, vx: 0, vy: 0, lastTap: 0, tapX: 0, tapY: 0 });
  const pinch = useRef<{ d: number; mx: number; my: number } | null>(null);
  const local = (e: any): Ptr => { const r = e.currentTarget?.getBoundingClientRect?.() || { left: 0, top: 0 }; return { x: e.clientX - r.left, y: e.clientY - r.top }; };
  const two = () => { const [a, b] = [...ptrs.current.values()]; return { d: Math.hypot(a.x - b.x, a.y - b.y), mx: (a.x + b.x) / 2, my: (a.y + b.y) / 2 }; };
  const R = M.fitRadius(W, H);
  // the wheel: React registers wheel listeners as passive at the root (React 19), so preventDefault there does nothing
  // and a trackpad pinch (ctrl + wheel) zooms the whole page too; a listener of our own, not passive, on the element
  const box = useRef<any>(null);
  const onWheel = useRef<(e: any) => void>(() => {});
  useEffect(() => {
    onWheel.current = (e: any) => {
      e.preventDefault();
      stopAll();
      const r = box.current?.getBoundingClientRect?.() || { left: 0, top: 0 };
      cam.current = M.zoomAbout(cam.current, cam.current.lz - e.deltaY * (e.ctrlKey ? 0.01 : 0.0015), e.clientX - r.left - W / 2, e.clientY - r.top - H / 2, R);
      redraw(); settle(cam.current);
    };
  });
  const hasCanvas = W > 0 && H > 0 && nodes.length > 0;
  useEffect(() => {
    const el = box.current;
    if (!el?.addEventListener) return;
    const f = (e: any) => onWheel.current(e);
    el.addEventListener('wheel', f, { passive: false });
    return () => el.removeEventListener('wheel', f);
  }, [hasCanvas]);
  const handlers = {
    onPointerDown: (e: any) => {
      const p = local(e);
      ptrs.current.set(e.pointerId, p);
      stopAll();
      if (ptrs.current.size === 1) Object.assign(drag.current, { x: p.x, y: p.y, t: now(), moved: false, vx: 0, vy: 0 });
      if (ptrs.current.size === 2) { pinch.current = two(); drag.current.moved = true; }
    },
    onPointerMove: (e: any) => {
      if (!ptrs.current.has(e.pointerId)) return;
      const p = local(e);
      ptrs.current.set(e.pointerId, p);
      if (ptrs.current.size >= 2 && pinch.current) {
        const t = two(), b = pinch.current;
        let c = M.zoomAbout(cam.current, cam.current.lz + Math.log(Math.max(0.001, t.d / Math.max(1, b.d))), t.mx - W / 2, t.my - H / 2, R);
        c = M.panBy(c, t.mx - b.mx, t.my - b.my, R);
        cam.current = c; pinch.current = t; redraw();
        return;
      }
      const d = drag.current;
      const dx = p.x - d.x, dy = p.y - d.y;
      // capture the pointer only once this is a drag: capturing on pointerdown retargets pointerup to this view
      if (!d.moved && Math.abs(dx) + Math.abs(dy) > 3) { d.moved = true; e.currentTarget?.setPointerCapture?.(e.pointerId); }
      if (!d.moved) return;
      const t = now(), dt = Math.max(1, t - d.t);
      d.vx = 0.7 * (dx / dt) * 1000 + 0.3 * d.vx; d.vy = 0.7 * (dy / dt) * 1000 + 0.3 * d.vy;
      Object.assign(d, { x: p.x, y: p.y, t });
      cam.current = M.orbitBy(cam.current, dx, dy, W);
      redraw();
    },
    onPointerUp: (e: any) => {
      const had = ptrs.current.size;
      ptrs.current.delete(e.pointerId);
      if (had >= 2) { pinch.current = null; if (ptrs.current.size === 1) { const [q] = [...ptrs.current.values()]; Object.assign(drag.current, { x: q.x, y: q.y, t: now(), vx: 0, vy: 0 }); } settle(cam.current); return; }
      const d = drag.current;
      if (!d.moved) {
        const p = local(e), t = now();
        if (t - d.lastTap < 300 && Math.hypot(p.x - d.tapX, p.y - d.tapY) < 24) { d.lastTap = 0; goTo(M.doubleTapTarget(cam.current, p.x - W / 2, p.y - H / 2, R)); return; }
        Object.assign(d, { lastTap: t, tapX: p.x, tapY: p.y });
        tapAt(p.x, p.y);
        return;
      }
      if (!reduce && now() - d.t < 80) { const k = M.radPerPx(W); vel.current = { yaw: -d.vx * k, pitch: d.vy * k }; setMotion(true); }
      settle(cam.current);
    },
    onPointerCancel: (e: any) => { ptrs.current.delete(e.pointerId); pinch.current = null; },
  };

  const command = useCallback((name: string) => {
    stopAll();
    const c = cam.current, step = Math.PI / 12, r = M.fitRadius(W, H);
    if (name === 'zoomIn') goTo(M.zoomAbout(c, c.lz + Math.LN2, 0, 0, r));
    else if (name === 'zoomOut') goTo(M.zoomAbout(c, c.lz - Math.LN2, 0, 0, r));
    else if (name === 'home') goTo(M.homeFrom(c));
    else if (name === 'turnLeft') goTo({ ...c, yaw: c.yaw + step });
    else if (name === 'turnRight') goTo({ ...c, yaw: c.yaw - step });
  }, [W, H, goTo, stopAll]);
  const fly = useCallback((c: M.CloudCluster) => { stopAll(); goTo(M.flyTo(cam.current, c, spread[clusters.indexOf(c)] ?? 0.3)); }, [clusters, spread, goTo, stopAll]);
  const placed = m.data?.counts.placed || 0;

  return (
    <CloudFrame
      st={{ data: m.data, error: m.rawError ? String(m.rawError) : null, reload: m.reload, spinning, setSpinning, reduce, focused: focus ? nodes[byId.get(focus) ?? -1] || null : null, setFocus, fly, moved, limit, command, onLinks }}
      onSize={(w, h) => setSize({ w, h })}
      canvas={(w, h) => (
        <View ref={box} testID="cloud-canvas" accessibilityRole="image" accessibilityLabel={`Cloud map: ${placed} bits in ${clusters.length} groups`}
          style={{ width: w, height: h, overflow: 'hidden', backgroundColor: C.bgDeep, touchAction: 'none', cursor: 'grab' } as any} {...(handlers as any)}>
          <Svg width={w} height={h}>
            {view?.paths.map((p) => <Path key={p.key} d={p.path} fill={p.color} fillOpacity={p.alpha} />)}
            {view?.ring && <Path d={view.ring} fill="none" stroke={C.amber} strokeWidth={2.5} />}
            {view?.names.map((l) => (
              <React.Fragment key={l.i}>
                <SvgText x={l.x} y={l.y} fontSize={12} fontWeight="600" fontFamily={F.semi} fill="none" stroke={C.bg} strokeWidth={5} strokeLinejoin="round" strokeOpacity={l.alpha}>{l.text}</SvgText>
                <SvgText x={l.x} y={l.y} fontSize={12} fontWeight="600" fontFamily={F.semi} fill={C.text} fillOpacity={l.alpha}>{l.text}</SvgText>
              </React.Fragment>
            ))}
          </Svg>
          {view?.labels.map((l) => (
            <Text key={l.id} pointerEvents="none" numberOfLines={1} maxFontSizeMultiplier={1.2}
              style={{ position: 'absolute', left: l.x - 70, top: l.y - 9, width: 140, textAlign: 'center', fontFamily: F.semi, fontSize: 12, color: C.text, opacity: l.alpha, textShadowColor: C.bg, textShadowRadius: 6 }}>
              {l.label}
            </Text>
          ))}
        </View>
      )}
    />
  );
}
