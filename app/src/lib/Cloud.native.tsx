// Bits › Map › the cloud on the phone: every bit placed by what it SAYS (GET /api/map/cloud: x, y, z in [-1, 1] from a
// PCA of its vector, a named k-means group), drawn with Skia on the GPU at device resolution, seen through an orbit
// camera (rules/cloudMath; spec docs/design/SPEC-2026-11-02-cloud-3d.md, owner 2026-11-02: "rotate it and zoom into
// different spots, this one gets blurry"). Zoom moves the camera, never scales the view, so dots and names stay crisp.
// One finger orbits (momentum), a pinch zooms about the fingers and its midpoint pans, a double tap zooms in (at 12×,
// back to the whole map), a double-tap-and-drag zooms one-handed, a two-finger tap zooms out, a tap selects a dot or
// flies to a group's name. Everything per frame runs on the UI thread: the camera lives in shared values, the gestures'
// callbacks are inline worklets, the picture is rebuilt in a worklet; JS hears only gesture ends and taps.
// Every function a worklet calls carries 'worklet' (the build-16 crash; tests/cloudMath.test.mjs pins it).
// The chrome (states, card, chips, zoom buttons) is lib/CloudChrome, shared with the web's SVG version (lib/Cloud).
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { AccessibilityInfo, PixelRatio, StyleSheet, View } from 'react-native';
import { router } from 'expo-router';
import { Canvas, Picture, Skia, StrokeJoin, createPicture, matchFont } from '@shopify/react-native-skia';
import type { SkFont, SkPaint } from '@shopify/react-native-skia';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import Animated, { Easing, cancelAnimation, runOnJS, runOnUI, useAnimatedStyle, useDerivedValue, useFrameCallback, useSharedValue, withDecay, withTiming } from 'react-native-reanimated';
import type { FrameInfo, SharedValue } from 'react-native-reanimated';
import { useApi } from './api';
import { CloudFrame, camValue, useInFront } from './CloudChrome';
import { haptics } from './haptics';
import { slotColor } from './Galaxy';
import * as M from '@/rules/cloudMath';
import { C, F } from './theme';
import { useReduceMotion } from './ui';

const RAD_PER_S = 0.12;
/** Double-tap-and-drag: 100 pt of drag doubles or halves the zoom. */
const ZOOM_PER_PT = Math.LN2 / 100;
const LEVELS = 16;

/** The dots as flat arrays the worklets read (copied to the UI thread once per payload, no objects per frame). */
type Flat = { n: number; x: Float32Array; y: Float32Array; z: Float32Array; cluster: Uint16Array; names: string[]; groups: number };

function flatten(nodes: M.CloudNode[]): Flat {
  const n = nodes.length;
  const x = new Float32Array(n), y = new Float32Array(n), z = new Float32Array(n), cluster = new Uint16Array(n);
  let groups = 1;
  const names: string[] = [];
  nodes.forEach((d, i) => { x[i] = d.x; y[i] = d.y; z[i] = d.z; cluster[i] = d.cluster; groups = Math.max(groups, d.cluster + 1); names.push(M.shortLabel(d.short_name || d.title || '')); });
  return { n, x, y, z, cluster, names, groups };
}

/** One paint per group × 16 alpha levels, made once on JS and drawn from on the UI thread (no Paint per frame). */
function makePaints(groups: number): SkPaint[] {
  const out: SkPaint[] = [];
  for (let c = 0; c < groups; c++) {
    const col = Skia.Color(slotColor(c % 8));
    for (let q = 0; q < LEVELS; q++) {
      const p = Skia.Paint();
      p.setAntiAlias(true);
      p.setColor(col);
      p.setAlphaf((q + 0.5) / LEVELS);
      out.push(p);
    }
  }
  return out;
}

type Cams = { yaw: SharedValue<number>; pitch: SharedValue<number>; tx: SharedValue<number>; ty: SharedValue<number>; tz: SharedValue<number>; lz: SharedValue<number> };

export function Cloud({ onLinks }: { onLinks: () => void }) {
  const reduce = useReduceMotion();
  const inFront = useInFront();
  const m = useApi<M.CloudMap>('/api/map/cloud?cap=1600', { poll: 0 });
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [spinning, setSpinning] = useState(!reduce);
  const [focus, setFocus] = useState<string | null>(null);
  const [moved, setMoved] = useState(false);
  const [a11yValue, setA11yValue] = useState('Whole map');
  const [limit, setLimit] = useState<'max' | 'min' | null>(null);

  // the camera: six shared values on the UI thread
  const yaw = useSharedValue(0), pitch = useSharedValue(0), tx = useSharedValue(0), ty = useSharedValue(0), tz = useSharedValue(0), lz = useSharedValue(0);
  const cams: Cams = useMemo(() => ({ yaw, pitch, tx, ty, tz, lz }), [yaw, pitch, tx, ty, tz, lz]);
  const spin = useSharedValue(0), rm = useSharedValue(reduce ? 1 : 0);
  const pinching = useSharedValue(0), pinched = useSharedValue(0), lastN = useSharedValue(1), mode = useSharedValue(0);
  const lastUpT = useSharedValue(0), lastUpX = useSharedValue(0), lastUpY = useSharedValue(0);
  const pfx = useSharedValue(0), pfy = useSharedValue(0), buzzed = useSharedValue(0), panned = useSharedValue(0);
  const focusIndex = useSharedValue(-1);

  const nodes = useMemo(() => m.data?.nodes || [], [m.data]);
  const clusters = useMemo(() => m.data?.clusters || [], [m.data]);
  const flat = useMemo(() => flatten(nodes), [nodes]);
  const spread = useMemo(() => M.spreads(nodes, clusters), [nodes, clusters]);
  const byId = useMemo(() => new Map(nodes.map((d, i) => [d.id, i])), [nodes]);
  const paints = useMemo(() => makePaints(flat.groups), [flat.groups]);
  const textC = C.text, bgC = C.bg, amber = C.amber;
  const ink = useMemo(() => {
    const ring = Skia.Paint(); ring.setAntiAlias(true); ring.setStyle(1); ring.setStrokeWidth(2.5); ring.setColor(Skia.Color(amber));
    const fill = Skia.Paint(); fill.setAntiAlias(true); fill.setColor(Skia.Color(textC));
    // the halo: a 5 pt stroke centred on the outline shows 2.5 pt outside it; round joins so corners never spike
    const halo = Skia.Paint(); halo.setAntiAlias(true); halo.setStyle(1); halo.setStrokeWidth(5); halo.setStrokeJoin(StrokeJoin.Round); halo.setColor(Skia.Color(bgC));
    return { ring, fill, halo };
  }, [textC, bgC, amber]);
  // bit names: the system font at 12 pt semibold, Dynamic Type up to 1.2× (HIG.md §3.10's cap for map labels)
  const font: SkFont = useMemo(() => matchFont({ fontFamily: 'System', fontSize: 12 * Math.min(PixelRatio.getFontScale(), 1.2), fontWeight: '600' }), []);
  // each name's drawn width, measured once per payload: a tap on a name hits the text, not a fixed 160 pt box
  const nameW = useMemo(() => Float32Array.from(flat.names, (t) => font.measureText(t).width), [flat, font]);
  const W = size.w, H = size.h;
  const turning = M.shouldSpin({ spinning, reduceMotion: reduce, nodes: flat.n, w: W, h: H, focused: inFront });

  useEffect(() => { rm.value = reduce ? 1 : 0; }, [reduce, rm]);
  useEffect(() => { spin.value = turning ? 1 : 0; }, [turning, spin]);
  useEffect(() => { focusIndex.value = focus ? (byId.get(focus) ?? -1) : -1; }, [focus, byId, focusIndex]);

  // the slow turn: UI thread, every frame, nothing crosses to JS. useFrameCallback reads its second argument only
  // when it first registers (react-native-reanimated src/hook/useFrameCallback.ts), so the turn is switched with
  // setActive: passing `turning` there (builds 17-18) left it off for good whenever the first render had no cloud yet.
  // The callback is memoised: a new function each render re-registers it and restarts the frame clock (a one-frame
  // pause per render, FrameCallbackRegistryUI.ts).
  const turnTick = useCallback((info: FrameInfo) => {
    'worklet';
    if (!spin.value) return;
    yaw.value = M.wrapAngle(yaw.value + (RAD_PER_S * (info.timeSincePreviousFrame ?? 0)) / 1000);
  }, [spin, yaw]);
  const frame = useFrameCallback(turnTick, false);
  useEffect(() => { frame.setActive(turning); }, [turning, frame]);

  // JS hears about the camera only when a gesture or a flight ends: the Whole map button and VoiceOver's value
  const report = useCallback((cx: number, cy: number, cz: number, clz: number, cpitch: number, announce: boolean) => {
    const home = M.isHome({ yaw: 0, pitch: cpitch, tx: cx, ty: cy, tz: cz, lz: clz });
    const v = camValue(clz, cx, cy, cz, clusters, home);
    setMoved(!home);
    setA11yValue(v);
    setLimit(clz >= M.LZ_MAX - 0.01 ? 'max' : clz <= M.LZ_MIN + 0.01 ? 'min' : null);
    // queued, so the custom action's own feedback does not cut it off
    if (announce) AccessibilityInfo.announceForAccessibilityWithOptions(v, { queue: true });
  }, [clusters]);
  const stopTurning = useCallback(() => setSpinning(false), []);
  const say = useCallback((t: string) => AccessibilityInfo.announceForAccessibilityWithOptions(t, { queue: true }), []);
  const buzz = useCallback(() => haptics.selection(), []);
  // plain functions, not useCallback: they reach worklets declared below, and the gestures are rebuilt every render
  // anyway, so each render's closures are the current ones (no stale `clusters` in VoiceOver's value)
  const onPick = (kind: number, idx: number) => {
    if (kind === 1) { const c = clusters[idx]; if (c) runOnUI(flyWorklet)(c.cx, c.cy, c.cz, spread[idx] ?? 0.3); return; }
    if (kind !== 0 || idx < 0 || idx >= nodes.length) { setFocus(null); return; }
    const id = nodes[idx].id;
    haptics.selection();
    if (id === focus) { router.push(`/joke/${encodeURIComponent(id)}` as any); return; }
    setFocus(id);
  };

  function camOf(): M.Cam {
    'worklet';
    return { yaw: yaw.value, pitch: pitch.value, tx: tx.value, ty: ty.value, tz: tz.value, lz: lz.value };
  }
  function setCam(c: M.Cam) {
    'worklet';
    yaw.value = c.yaw; pitch.value = c.pitch; tx.value = c.tx; ty.value = c.ty; tz.value = c.tz; lz.value = c.lz;
  }
  function stopAll() {
    'worklet';
    cancelAnimation(yaw); cancelAnimation(pitch); cancelAnimation(tx); cancelAnimation(ty); cancelAnimation(tz); cancelAnimation(lz);
    if (spin.value) { spin.value = 0; runOnJS(stopTurning)(); }
  }
  function reportNow(announce: boolean) {
    'worklet';
    runOnJS(report)(tx.value, ty.value, tz.value, lz.value, pitch.value, announce);
  }
  /** Animate to a camera (350 ms, ease-out; any touch cancels); under Reduce Motion it jumps. */
  function animateTo(c: M.Cam, announce: boolean) {
    'worklet';
    if (rm.value) setCam(c);
    else {
      const cfg = { duration: 350, easing: Easing.out(Easing.cubic) };
      yaw.value = withTiming(c.yaw, cfg); pitch.value = withTiming(c.pitch, cfg);
      tx.value = withTiming(c.tx, cfg); ty.value = withTiming(c.ty, cfg); tz.value = withTiming(c.tz, cfg); lz.value = withTiming(c.lz, cfg);
    }
    runOnJS(report)(c.tx, c.ty, c.tz, c.lz, c.pitch, announce);
  }
  function flyWorklet(cx: number, cy: number, cz: number, sp: number) {
    'worklet';
    stopAll();
    animateTo(M.flyTo(camOf(), { id: 0, label: '', size: 0, cx, cy, cz }, sp), false);
  }
  /** The buttons and VoiceOver's actions: zoom in/out about the centre, whole map, turn, tilt. */
  function act(name: string, w: number, h: number) {
    'worklet';
    stopAll();
    const c = camOf();
    const R = M.fitRadius(w, h);
    const step = Math.PI / 12;
    // at a limit VoiceOver says so instead of repeating the same value (the buttons are disabled there)
    if (name === 'zoomIn' && c.lz >= M.LZ_MAX - 0.01) runOnJS(say)('Closest zoom');
    else if (name === 'zoomOut' && c.lz <= M.LZ_MIN + 0.01) runOnJS(say)('Widest zoom');
    else if (name === 'zoomIn') animateTo(M.zoomAbout(c, c.lz + Math.LN2, 0, 0, R), true);
    else if (name === 'zoomOut') animateTo(M.zoomAbout(c, c.lz - Math.LN2, 0, 0, R), true);
    else if (name === 'home') animateTo(M.homeFrom(c), true);
    else if (name === 'turnLeft') animateTo({ ...c, yaw: c.yaw + step }, true);
    else if (name === 'turnRight') animateTo({ ...c, yaw: c.yaw - step }, true);
    else if (name === 'tiltUp') animateTo({ ...c, pitch: M.clampPitch(c.pitch - step) }, true);
    else if (name === 'tiltDown') animateTo({ ...c, pitch: M.clampPitch(c.pitch + step) }, true);
  }
  /** A tap (M.resolveTap: a dot under the finger, a bit's name, a group's name, the nearest dot; nothing = clear),
   *  hit-tested on the UI thread against the projection on screen right now (no view.scale), then one call to JS. */
  function pick(px: number, py: number) {
    'worklet';
    const n = flat.n;
    const cam = camOf();
    const pj = M.newProj(n);
    M.projectInto(n, flat.x, flat.y, flat.z, cam, W, H, pj);
    const t = M.resolveTap(pj, M.depthOrder(pj.depth, n), n, W, H, cam, focusIndex.value, px, py, nameW, clusters);
    runOnJS(onPick)(t.kind, t.idx);
  }

  // the picture: one draw list per camera change, built in a worklet from the flat arrays
  const picture = useDerivedValue(() => {
    'worklet';
    const n = flat.n;
    const cam = { yaw: yaw.value, pitch: pitch.value, tx: tx.value, ty: ty.value, tz: tz.value, lz: lz.value };
    const pj = M.newProj(n);
    M.projectInto(n, flat.x, flat.y, flat.z, cam, W, H, pj);
    const order = M.depthOrder(pj.depth, n);
    const fi = focusIndex.value;
    const zoom = Math.exp(cam.lz);
    const la = M.bitLabelAlpha(zoom);
    const picked = la > 0 ? M.pickLabels(pj, order, n, W, H, zoom, fi) : null;
    return createPicture((canvas) => {
      for (let k = 0; k < n; k++) {
        const i = order[k];
        const a = pj.alpha[i];
        if (!(a > 0)) continue; // also NaN (a node without x, y or z): an undefined paint throws on the UI thread
        const pt = paints[flat.cluster[i] * LEVELS + Math.min(LEVELS - 1, Math.floor(a * LEVELS))];
        if (!pt) continue;
        canvas.drawCircle(pj.sx[i], pj.sy[i], pj.r[i], pt);
      }
      if (fi >= 0 && fi < n && pj.alpha[fi] > 0) canvas.drawCircle(pj.sx[fi], pj.sy[fi], pj.r[fi] + 5, ink.ring);
      if (picked) {
        for (let k = 0; k < picked.length && picked[k] >= 0; k++) {
          const i = picked[k];
          const a = la * pj.fade[i];
          ink.halo.setAlphaf(a); ink.fill.setAlphaf(a);
          const lx = pj.sx[i] + pj.r[i] + 4, ly = pj.sy[i] + 4;
          canvas.drawText(flat.names[i], lx, ly, ink.halo, font);
          canvas.drawText(flat.names[i], lx, ly, ink.fill, font);
        }
      }
    }, { width: Math.max(1, W), height: Math.max(1, H) });
  }, [flat, W, H, paints, ink, font]);

  // gestures: every callback inline in its Gesture chain, so the worklets plugin makes it a worklet
  // (react-native-gesture-handler 3.3's builder API; a handler passed by name would run on JS and mixing is an error)
  const orbit = Gesture.Pan()
    .onBegin((e) => {
      stopAll();
      const dx = e.x - lastUpX.value, dy = e.y - lastUpY.value;
      mode.value = Date.now() - lastUpT.value < 350 && dx * dx + dy * dy < 1600 ? 1 : 0; // a tap just before: zoom drag
      lastN.value = e.numberOfPointers;
      pinched.value = 0; panned.value = 0; buzzed.value = 0;
    })
    .onStart(() => { panned.value = 1; })
    .onChange((e) => {
      // one finger orbits; a second finger hands over to the pinch, and the first update back at one finger re-bases
      // (the touch centre jumps when the count changes, which would read as a spin)
      if (pinching.value || e.numberOfPointers !== 1) { lastN.value = e.numberOfPointers; return; }
      if (lastN.value !== 1) { lastN.value = 1; return; }
      if (mode.value === 1) {
        const want = lz.value - e.changeY * ZOOM_PER_PT;
        if ((want > M.LZ_MAX || want < M.LZ_MIN) && !buzzed.value) { buzzed.value = 1; runOnJS(buzz)(); }
        setCam(M.zoomAbout(camOf(), want, lastUpX.value - W / 2, lastUpY.value - H / 2, M.fitRadius(W, H)));
      }
      else { const c = M.orbitBy(camOf(), e.changeX, e.changeY, W); yaw.value = c.yaw; pitch.value = c.pitch; }
    })
    .onEnd((e, ok) => {
      if (!ok || mode.value !== 0 || pinched.value || rm.value) return; // a cancelled drag (a call) does not fling
      const k = M.radPerPx(W);
      yaw.value = withDecay({ velocity: -e.velocityX * k });
      pitch.value = withDecay({ velocity: e.velocityY * k, clamp: [-M.PITCH_LIMIT, M.PITCH_LIMIT] });
    })
    .onFinalize((e) => {
      // a touch that never became a drag was a tap: remember it for double-tap-and-drag; a drag (ended or cancelled)
      // tells JS where the camera is
      if (!panned.value) { lastUpT.value = Date.now(); lastUpX.value = e.x; lastUpY.value = e.y; return; }
      lastUpT.value = 0;
      reportNow(false);
    });
  const pinch = Gesture.Pinch()
    .onBegin(() => { stopAll(); })
    .onStart((e) => { pinching.value = 1; pinched.value = 1; pfx.value = e.focalX; pfy.value = e.focalY; buzzed.value = 0; })
    .onChange((e) => {
      const R = M.fitRadius(W, H);
      const want = lz.value + Math.log(Math.max(0.001, e.scaleChange));
      if ((want > M.LZ_MAX || want < M.LZ_MIN) && !buzzed.value) { buzzed.value = 1; runOnJS(buzz)(); }
      const c = M.panBy(M.zoomAbout(camOf(), want, e.focalX - W / 2, e.focalY - H / 2, R), e.focalX - pfx.value, e.focalY - pfy.value, R);
      pfx.value = e.focalX; pfy.value = e.focalY;
      setCam(c);
    })
    .onFinalize(() => { if (pinching.value) { pinching.value = 0; reportNow(false); } });
  const twoTap = Gesture.Tap().minPointers(2)
    .onEnd((_e, ok) => { if (ok) act('zoomOut', W, H); });
  const doubleTap = Gesture.Tap().numberOfTaps(2).maxDistance(24)
    .onEnd((e, ok) => { if (ok) { stopAll(); animateTo(M.doubleTapTarget(camOf(), e.x - W / 2, e.y - H / 2, M.fitRadius(W, H)), false); } });
  const tap = Gesture.Tap().maxDistance(12)
    .onEnd((e, ok) => { if (ok) pick(e.x, e.y); });
  // the two-finger tap first: a one-pointer tap also succeeds with two fingers (RNTapHandler.m)
  const gesture = Gesture.Race(Gesture.Simultaneous(orbit, pinch), Gesture.Exclusive(twoTap, doubleTap, tap));

  const fly = (c: M.CloudCluster) => runOnUI(flyWorklet)(c.cx, c.cy, c.cz, spread[clusters.indexOf(c)] ?? 0.3);
  const command = (name: string) => runOnUI(act)(name, W, H);
  const focused = focus ? nodes[byId.get(focus) ?? -1] || null : null;
  const placed = m.data?.counts.placed || 0;

  return (
    <CloudFrame
      st={{ data: m.data, error: m.rawError ? String(m.rawError) : null, reload: m.reload, spinning, setSpinning, reduce, focused, setFocus, fly, onLinks, moved, limit, command }}
      onSize={(w, h) => setSize({ w, h })}
      canvas={(w, h) => (
        <GestureDetector gesture={gesture}>
          <View testID="cloud-canvas" collapsable={false} style={{ width: w, height: h, overflow: 'hidden', backgroundColor: C.bgDeep }}
            accessible accessibilityRole="image" accessibilityLabel={`Cloud map: ${placed} bits in ${clusters.length} groups`}
            accessibilityValue={{ text: a11yValue }} accessibilityHint="The Links button reads every bit as a list."
            accessibilityActions={A11Y_ACTIONS} onAccessibilityAction={(e) => command(e.nativeEvent.actionName)}>
            <Canvas style={{ width: w, height: h }}>
              <Picture picture={picture} />
            </Canvas>
            {clusters.map((c) => <ClusterLabel key={c.id} c={c} cams={cams} w={w} h={h} />)}
          </View>
        </GestureDetector>
      )}
    />
  );
}

/** VoiceOver's alternatives to the gestures, the common task first (HIG/voiceover). */
const A11Y_ACTIONS = [
  { name: 'zoomIn', label: 'Zoom in' }, { name: 'zoomOut', label: 'Zoom out' }, { name: 'home', label: 'Whole map' },
  { name: 'turnLeft', label: 'Turn left' }, { name: 'turnRight', label: 'Turn right' },
  { name: 'tiltUp', label: 'Tilt up' }, { name: 'tiltDown', label: 'Tilt down' },
];

/** A group's name floating at its centre; position and fade follow the camera on the UI thread. */
function ClusterLabel({ c, cams, w, h }: { c: M.CloudCluster; cams: Cams; w: number; h: number }) {
  const style = useAnimatedStyle(() => {
    'worklet';
    const cam = { yaw: cams.yaw.value, pitch: cams.pitch.value, tx: cams.tx.value, ty: cams.ty.value, tz: cams.tz.value, lz: cams.lz.value };
    const p = M.projectPoint(c.cx, c.cy, c.cz, cam, w, h);
    const a = M.groupLabelAlpha(p.depth, p.fade, Math.exp(cam.lz), Math.hypot(p.sx - w / 2, p.sy - h / 2), Math.min(w, h));
    return { transform: [{ translateX: p.sx - 70 }, { translateY: p.sy - 9 }], opacity: a };
  });
  // colours at render time: C is swapped in place on a scheme change, and a StyleSheet made at module load keeps the old ones
  return <Animated.Text pointerEvents="none" numberOfLines={1} maxFontSizeMultiplier={1.2} style={[styles.label, { color: C.text, textShadowColor: C.bg }, style]}>{c.label}</Animated.Text>;
}

const styles = StyleSheet.create({
  label: { position: 'absolute', left: 0, top: 0, width: 140, textAlign: 'center', fontFamily: F.semi, fontSize: 12, textShadowRadius: 6 },
});
