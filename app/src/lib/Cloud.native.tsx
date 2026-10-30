// Bits › Map › the cloud on the phone (2026-10-29, owner: "bro that's horrible performance, let's use react-native-skia"):
// every bit placed by what it SAYS (GET /api/map/cloud: x, y, z in [-1, 1] from a PCA of its vector, a named k-means
// group), drawn with Skia on the GPU. The turn is a Reanimated frame callback: the angle lives in a shared value, the
// 1,600 projections and the picture are rebuilt in a worklet on the UI thread every frame, and the JS thread does
// nothing per frame (build 12's SVG version rebuilt ~130 KB of path text on the JS thread and starved every tap).
// Cluster names are RN Text with animated transforms (UI thread too). A tap reads the angle and finds the nearest
// dot on JS once. Pinch and pan come from lib/PanZoom. The chrome (states, card, chips) is lib/CloudChrome, shared
// with the web's SVG version (lib/Cloud).
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Pressable, StyleSheet, Text } from 'react-native';
import { router } from 'expo-router';
import { Canvas, Picture, Skia, createPicture } from '@shopify/react-native-skia';
import Animated, { type SharedValue, useAnimatedStyle, useDerivedValue, useFrameCallback, useSharedValue } from 'react-native-reanimated';
import { useApi } from './api';
import { CloudFrame, useInFront } from './CloudChrome';
import { haptics } from './haptics';
import { PanZoom } from './PanZoom';
import type { View_ } from './PanZoom';
import { slotColor } from './Galaxy';
import * as M from '@/rules/cloudMath';
import { C, F } from './theme';
import { useReduceMotion } from './ui';

const RAD_PER_S = 0.12;
const PAD = 28;

/** The dots as flat arrays the worklet reads (no objects per frame). */
type Flat = { n: number; x: Float32Array; y: Float32Array; z: Float32Array; cluster: Uint16Array; colors: Float32Array[] };

function flatten(nodes: M.CloudNode[]): Flat {
  const n = nodes.length;
  const x = new Float32Array(n), y = new Float32Array(n), z = new Float32Array(n), cluster = new Uint16Array(n);
  let maxCluster = 0;
  nodes.forEach((d, i) => { x[i] = d.x; y[i] = d.y; z[i] = d.z; cluster[i] = d.cluster; if (d.cluster > maxCluster) maxCluster = d.cluster; });
  const colors: Float32Array[] = [];
  for (let c = 0; c <= maxCluster; c++) colors.push(Skia.Color(slotColor(c % 8)));
  return { n, x, y, z, cluster, colors };
}

export function Cloud({ onLinks }: { onLinks: () => void }) {
  const reduce = useReduceMotion();
  const inFront = useInFront();
  const m = useApi<M.CloudMap>('/api/map/cloud?cap=1600', { poll: 0 });
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View_>({ x: 0, y: 0, scale: 1 });
  const [spinning, setSpinning] = useState(!reduce);
  const [focus, setFocus] = useState<string | null>(null);
  const theta = useSharedValue(0);
  const focusIndex = useSharedValue(-1);
  const nodes = m.data?.nodes || [];
  const flat = useMemo(() => flatten(nodes), [nodes]);
  const byId = useMemo(() => new Map(nodes.map((d, i) => [d.id, i])), [nodes]);
  const W = size.w, H = size.h;
  const turning = M.shouldSpin({ spinning, reduceMotion: reduce, nodes: flat.n, w: W, h: H, focused: inFront });

  // the turn: UI thread, every frame, nothing crosses to JS
  useFrameCallback((info) => {
    'worklet';
    const dt = info.timeSincePreviousFrame ?? 0;
    theta.value = M.wrapAngle(theta.value + (RAD_PER_S * dt) / 1000);
  }, turning);

  // the picture: one draw list per frame, built in a worklet from the flat arrays
  const picture = useDerivedValue(() => {
    'worklet';
    const th = theta.value;
    const n = flat.n;
    const R = Math.max(1, (Math.min(W, H) - PAD * 2) / 2);
    const cx = W / 2, cy = H / 2;
    const c = Math.cos(th), s = Math.sin(th);
    const sx = new Float32Array(n), sy = new Float32Array(n), depth = new Float32Array(n);
    const order = new Uint16Array(n);
    for (let i = 0; i < n; i++) {
      const xr = flat.x[i] * c - flat.z[i] * s;
      const zr = flat.x[i] * s + flat.z[i] * c;
      const d = Math.max(0, Math.min(1, (zr + 1) / 2));
      const pers = 0.82 + 0.36 * d;
      sx[i] = cx + xr * R * pers;
      sy[i] = cy - flat.y[i] * R * pers;
      depth[i] = d;
      order[i] = i;
    }
    // far dots first so near ones draw on top (a counting sort into 16 depth buckets: no comparator calls)
    const buckets = 16;
    const counts = new Uint16Array(buckets + 1);
    for (let i = 0; i < n; i++) counts[Math.min(buckets - 1, Math.floor(depth[i] * buckets)) + 1]++;
    for (let b = 0; b < buckets; b++) counts[b + 1] += counts[b];
    const sorted = new Uint16Array(n);
    for (let i = 0; i < n; i++) { const b = Math.min(buckets - 1, Math.floor(depth[i] * buckets)); sorted[counts[b]++] = i; }
    const fi = focusIndex.value;
    return createPicture((canvas) => {
      const paint = Skia.Paint();
      paint.setAntiAlias(true);
      for (let k = 0; k < n; k++) {
        const i = sorted[k];
        const d = depth[i];
        const col = flat.colors[flat.cluster[i]];
        paint.setColor(col);
        paint.setAlphaf(M.ALPHA_MIN + (1 - M.ALPHA_MIN) * d);
        canvas.drawCircle(sx[i], sy[i], M.R_MIN + (M.R_MAX - M.R_MIN) * d, paint);
      }
      if (fi >= 0 && fi < n) {
        const ring = Skia.Paint();
        ring.setAntiAlias(true);
        ring.setStyle(1); // stroke
        ring.setStrokeWidth(2.5);
        ring.setColor(Skia.Color(C.amber));
        canvas.drawCircle(sx[fi], sy[fi], M.R_MIN + (M.R_MAX - M.R_MIN) * depth[fi] + 5, ring);
      }
    }, { width: Math.max(1, W), height: Math.max(1, H) });
  }, [flat, W, H]);

  useEffect(() => { focusIndex.value = focus ? (byId.get(focus) ?? -1) : -1; }, [focus, byId, focusIndex]);

  const tap = useCallback((px: number, py: number) => {
    setSpinning(false);
    const dots = M.projectAll(nodes, W, H, theta.value);
    const hit = M.nearest(dots, px, py, 18 / Math.max(0.5, view.scale));
    if (!hit) return setFocus(null);
    haptics.selection();
    if (hit.id === focus) return router.push(`/joke/${encodeURIComponent(hit.id)}` as any);
    setFocus(hit.id);
  }, [nodes, W, H, theta, view.scale, focus]);
  const face = useCallback((c: M.CloudCluster) => { setSpinning(false); theta.value = M.faceAngle(c.cx, c.cz); setView({ x: 0, y: 0, scale: 1 }); }, [theta]);
  const turnBy = useCallback((d: number) => { setSpinning(false); theta.value = M.wrapAngle(theta.value + d); }, [theta]);
  const focused = focus ? nodes[byId.get(focus) ?? -1] || null : null;
  const placed = m.data?.counts.placed || 0;
  const groups = m.data?.clusters.length || 0;

  return (
    <CloudFrame
      st={{ data: m.data, error: m.rawError ? String(m.rawError) : null, reload: m.reload, spinning, setSpinning, reduce, focused, setFocus, face, turnBy, onLinks }}
      onSize={(w, h) => setSize({ w, h })}
      canvas={(w, h) => (
        <PanZoom view={view} onView={(v) => { setSpinning(false); setView(v); }} width={w} height={h} minScale={0.6} maxScale={4} testID="cloud-canvas">
          <Pressable testID="cloud-tap" accessibilityRole="image" accessibilityLabel={`${placed} bits as a cloud. ${groups} groups.`} onPress={(e) => tap(e.nativeEvent.locationX, e.nativeEvent.locationY)} style={{ width: w, height: h }}>
            <Canvas style={{ width: w, height: h }}>
              <Picture picture={picture} />
            </Canvas>
            {(m.data?.clusters || []).map((c) => <ClusterLabel key={c.id} c={c} theta={theta} w={w} h={h} />)}
          </Pressable>
        </PanZoom>
      )}
    />
  );
}

/** A group's name floating at its centre; its position and fade follow the turn on the UI thread. */
function ClusterLabel({ c, theta, w, h }: { c: M.CloudCluster; theta: SharedValue<number>; w: number; h: number }) {
  const style = useAnimatedStyle(() => {
    'worklet';
    const th = theta.value;
    const R = Math.max(1, (Math.min(w, h) - 56) / 2);
    const xr = c.cx * Math.cos(th) - c.cz * Math.sin(th);
    const zr = c.cx * Math.sin(th) + c.cz * Math.cos(th);
    const depth = Math.max(0, Math.min(1, (zr + 1) / 2));
    const pers = 0.82 + 0.36 * depth;
    return { transform: [{ translateX: w / 2 + xr * R * pers - 70 }, { translateY: h / 2 - c.cy * R * pers - 9 }], opacity: 0.35 + 0.65 * depth };
  });
  return (
    <Animated.Text pointerEvents="none" numberOfLines={1} maxFontSizeMultiplier={1.2} style={[styles.label, style]}>{c.label}</Animated.Text>
  );
}

const styles = StyleSheet.create({
  label: { position: 'absolute', left: 0, top: 0, width: 140, textAlign: 'center', fontFamily: F.semi, fontSize: 12, color: C.text, textShadowColor: C.bg, textShadowRadius: 6 },
});

// keep the import used on every platform build (Text is what Animated.Text wraps)
void Text;
