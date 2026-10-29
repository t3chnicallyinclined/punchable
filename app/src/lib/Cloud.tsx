// Bits › Map › the cloud on the web: every bit placed by what it SAYS (GET /api/map/cloud), drawn as SVG paths, one
// per cluster × depth band (lib/cloudMath), re-projected every ~33 ms while turning. The phone draws the same cloud
// with Skia on the GPU (lib/Cloud.native). The chrome (states, card, chips) is lib/CloudChrome.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Pressable, Text } from 'react-native';
import { router } from 'expo-router';
import Svg, { Path } from 'react-native-svg';
import { useApi } from './api';
import { CloudFrame, useInFront } from './CloudChrome';
import { haptics } from './haptics';
import { PanZoom } from './PanZoom';
import type { View_ } from './PanZoom';
import { slotColor } from './Galaxy';
import * as M from './cloudMath';
import { C, F } from './theme';
import { useReduceMotion } from './ui';

const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());

export function Cloud({ onLinks }: { onLinks: () => void }) {
  const reduce = useReduceMotion();
  const inFront = useInFront();
  const m = useApi<M.CloudMap>('/api/map/cloud?cap=1600', { poll: 0 });
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View_>({ x: 0, y: 0, scale: 1 });
  const [theta, setTheta] = useState(0);
  const [spinning, setSpinning] = useState(!reduce);
  const [focus, setFocus] = useState<string | null>(null);
  const W = size.w, H = size.h;
  const nodes = m.data?.nodes || [];
  const turning = M.shouldSpin({ spinning, reduceMotion: reduce, nodes: nodes.length, w: W, h: H, focused: inFront });
  useEffect(() => {
    if (!turning) return;
    let last = now();
    const t = setInterval(() => {
      const n = now();
      const step = M.spinStep(false, n - last);
      last = n;
      setTheta((th) => M.wrapAngle(th + step));
    }, M.SPIN_FRAME_MS);
    return () => clearInterval(t);
  }, [turning]);

  const dots = useMemo(() => (W && H ? M.projectAll(nodes, W, H, theta) : []), [nodes, W, H, theta]);
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const paths = useMemo(() => {
    const groups = new Map<string, { color: string; alpha: number; d: M.Dot[] }>();
    for (const d of dots) {
      const b = M.band(d.depth);
      const key = `${d.cluster}:${b}`;
      let g = groups.get(key);
      if (!g) { g = { color: slotColor(d.cluster % 8), alpha: M.BAND_ALPHA[b], d: [] }; groups.set(key, g); }
      g.d.push(d);
    }
    return [...groups.entries()].sort((a, b) => a[1].alpha - b[1].alpha).map(([key, g]) => ({ key, color: g.color, alpha: g.alpha, path: M.circlesPath(g.d) }));
  }, [dots]);
  const labels = useMemo(() => {
    if (!m.data || !W || !H) return [];
    const R = Math.max(1, (Math.min(W, H) - 56) / 2);
    return m.data.clusters.map((c) => {
      const t = M.turn(c.cx, c.cz, theta);
      const pers = 0.82 + 0.36 * t.depth;
      return { id: c.id, label: c.label, x: W / 2 + t.x * R * pers, y: H / 2 - c.cy * R * pers, depth: t.depth };
    });
  }, [m.data, W, H, theta]);
  const focusDot = focus ? dots.find((d) => d.id === focus) || null : null;
  const tap = useCallback((px: number, py: number) => {
    const hit = M.nearest(dots, px, py, 18 / Math.max(0.5, view.scale));
    setSpinning(false);
    if (!hit) return setFocus(null);
    haptics.selection();
    if (hit.id === focus) return router.push(`/joke/${encodeURIComponent(hit.id)}` as any);
    setFocus(hit.id);
  }, [dots, focus, view.scale]);
  const face = useCallback((c: M.CloudCluster) => { setSpinning(false); setTheta(M.faceAngle(c.cx, c.cz)); setView({ x: 0, y: 0, scale: 1 }); }, []);
  const turnBy = useCallback((d: number) => { setSpinning(false); setTheta((t) => M.wrapAngle(t + d)); }, []);
  const placed = m.data?.counts.placed || 0;
  const groups = m.data?.clusters.length || 0;

  return (
    <CloudFrame
      st={{ data: m.data, error: m.rawError ? String(m.rawError) : null, reload: m.reload, spinning, setSpinning, reduce, focused: focus ? byId.get(focus) || null : null, setFocus, face, turnBy, onLinks }}
      onSize={(w, h) => setSize({ w, h })}
      canvas={(w, h) => (
        <PanZoom view={view} onView={(v) => { setSpinning(false); setView(v); }} width={w} height={h} minScale={0.6} maxScale={4} testID="cloud-canvas">
          <Pressable testID="cloud-tap" accessibilityRole="image" accessibilityLabel={`${placed} bits as a cloud. ${groups} groups.`} onPress={(e) => tap(e.nativeEvent.locationX, e.nativeEvent.locationY)} style={{ width: w, height: h }}>
            <Svg width={w} height={h}>
              {paths.map((p) => <Path key={p.key} d={p.path} fill={p.color} fillOpacity={p.alpha} />)}
              {focusDot && <Path d={M.circlesPath([{ ...focusDot, r: focusDot.r + 5 }])} fill="none" stroke={C.amber} strokeWidth={2.5} />}
            </Svg>
            {labels.map((l) => (
              <Text key={l.id} pointerEvents="none" numberOfLines={1} maxFontSizeMultiplier={1.2}
                style={{ position: 'absolute', left: l.x - 70, top: l.y - 9, width: 140, textAlign: 'center', fontFamily: F.semi, fontSize: 12, color: C.text, opacity: 0.35 + 0.65 * l.depth, textShadowColor: C.bg, textShadowRadius: 6 }}>
                {l.label}
              </Text>
            ))}
          </Pressable>
        </PanZoom>
      )}
    />
  );
}
