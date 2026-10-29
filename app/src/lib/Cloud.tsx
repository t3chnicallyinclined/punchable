// Bits › Map › the cloud (2026-10-29, owner: "an AI-visualized 3D cloud map"): every bit placed by what it SAYS.
// GET /api/map/cloud (api-dev/cloud_api.py) gives each bit x, y, z in [-1, 1] from a PCA of its bge-small vector and a
// k-means cluster named by its titles' words. Here: a slow turn about the vertical axis (paused by a touch, off under
// Reduce Motion), depth as size and fade, pinch and pan from lib/PanZoom, a tap selects the nearest dot (a card with
// Open · To a set), cluster names floating at their centres, and a legend of clusters that turns one to the front.
// Drawn with react-native-svg as one path per cluster × depth band (lib/cloudMath), so 1,600 dots are ~40 elements.
// "Links" switches to the older map (lib/Galaxy), where nearness means a stored link, not meaning.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { Stack, router } from 'expo-router';
import Svg, { Path } from 'react-native-svg';
import { useApi } from './api';
import { useBoards } from './boards';
import { haptics } from './haptics';
import { PanZoom } from './PanZoom';
import type { View_ } from './PanZoom';
import { slotColor } from './Galaxy';
import * as M from './cloudMath';
import { C, F } from './theme';
import { Btn, Empty, ErrorCard, Skeleton, T, TextBtn, useReduceMotion, useWide } from './ui';
import { Screen } from './Screen';

const native = Platform.OS !== 'web';
const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
const raf = (f: () => void) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(f) : setTimeout(f, 16));

export function Cloud({ onLinks }: { onLinks: () => void }) {
  const wide = useWide();
  const reduce = useReduceMotion();
  const boards = useBoards();
  const m = useApi<M.CloudMap>('/api/map/cloud?cap=1600', { poll: 0 });
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View_>({ x: 0, y: 0, scale: 1 });
  const [theta, setTheta] = useState(0);
  const [spinning, setSpinning] = useState(!reduce);
  const [focus, setFocus] = useState<string | null>(null);
  const thetaRef = useRef(theta);
  thetaRef.current = theta;
  const W = size.w, H = size.h;

  // the slow turn, until he touches something
  useEffect(() => {
    if (!spinning || reduce) return;
    let alive = true, last = now();
    const tick = () => {
      if (!alive) return;
      const t = now();
      setTheta((th) => th + M.spinStep(false, t - last));
      last = t;
      raf(tick);
    };
    raf(tick);
    return () => { alive = false; };
  }, [spinning, reduce]);

  const dots = useMemo(() => (m.data && W && H ? M.projectAll(m.data.nodes, W, H, theta) : []), [m.data, W, H, theta]);
  const byId = useMemo(() => new Map((m.data?.nodes || []).map((n) => [n.id, n])), [m.data]);
  const clusterById = useMemo(() => new Map((m.data?.clusters || []).map((c) => [c.id, c])), [m.data]);
  // one path per cluster × depth band
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
      return { id: c.id, label: c.label, size: c.size, x: W / 2 + t.x * R * pers, y: H / 2 - c.cy * R * pers, depth: t.depth };
    });
  }, [m.data, W, H, theta]);
  const focused = focus ? byId.get(focus) || null : null;
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
  const turnBy = (d: number) => { setSpinning(false); setTheta((t) => t + d); };

  const toolbar = native ? (
    <Stack.Toolbar placement="right">
      <Stack.Toolbar.Button icon={spinning ? 'pause' : 'rotate.3d'} accessibilityLabel={spinning ? 'Stop turning' : 'Turn the cloud'} onPress={() => setSpinning((v) => !v && !reduce)}>{spinning ? 'Still' : 'Turn'}</Stack.Toolbar.Button>
      <Stack.Toolbar.Button icon="point.3.connected.trianglepath.dotted" accessibilityLabel="The links map instead" onPress={onLinks}>Links</Stack.Toolbar.Button>
    </Stack.Toolbar>
  ) : null;
  const webRight = !native ? (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, paddingRight: 8 }}>
      <TextBtn testID="cloud-turn" label={spinning ? 'Still' : 'Turn'} onPress={() => setSpinning((v) => !v && !reduce)} />
      <TextBtn testID="cloud-links" label="Links" onPress={onLinks} />
    </View>
  ) : undefined;
  const counts = m.data ? `${m.data.counts.placed} bits by meaning · ${m.data.clusters.length} groups${m.data.counts.placed < m.data.counts.jokes ? ` · ${m.data.counts.jokes - m.data.counts.placed} not placed yet` : ''}` : '';

  return (
    <Screen title="Map" back backLabel="Bits" testID="cloud" scroll={false} right={webRight}>
      {toolbar}
      {m.rawError && !m.data ? <View style={{ padding: 16, gap: 10 }}><ErrorCard error={m.rawError} onRetry={m.reload} /><Btn small label="Show the links map" onPress={onLinks} /></View> : null}
      {!m.data && !m.rawError && <View style={{ padding: 16 }}><Skeleton lines={5} height={18} testID="cloud-skeleton" /></View>}
      {m.data && !m.data.nodes.length && <Empty text="Nothing placed yet: the vectors are still being made. Try again in a minute." />}
      {m.data && m.data.nodes.length > 0 && (
        <View style={{ flex: 1 }}>
          <View style={{ paddingHorizontal: 16, paddingTop: 8, gap: 6 }}>
            <T v="small" testID="cloud-counts">{counts}{!focus ? ' · near = says the same kind of thing · tap a dot' : ''}</T>
          </View>
          <View style={{ flex: 1 }} onLayout={(e) => setSize({ w: e.nativeEvent.layout.width, h: e.nativeEvent.layout.height })}>
            {W > 0 && H > 0 && (
              <PanZoom view={view} onView={(v) => { setSpinning(false); setView(v); }} width={W} height={H} minScale={0.6} maxScale={4} testID="cloud-canvas">
                <Pressable testID="cloud-tap" accessibilityRole="image" accessibilityLabel={`${m.data.counts.placed} bits as a cloud. ${m.data.clusters.length} groups.`} onPress={(e) => tap(e.nativeEvent.locationX, e.nativeEvent.locationY)} style={{ width: W, height: H }}>
                  <Svg width={W} height={H}>
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
            <View style={{ position: 'absolute', right: 12, top: 12, gap: 6 }}>
              <Pressable testID="cloud-left" accessibilityRole="button" accessibilityLabel="Turn left" onPress={() => turnBy(-Math.PI / 12)} style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.text, fontSize: 18 }}>⟲</Text></Pressable>
              <Pressable testID="cloud-right" accessibilityRole="button" accessibilityLabel="Turn right" onPress={() => turnBy(Math.PI / 12)} style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.text, fontSize: 18 }}>⟳</Text></Pressable>
            </View>
            {focused && (
              <View testID="cloud-card" style={{ position: 'absolute', left: 12, right: 12, bottom: 12, maxWidth: 520, padding: 12, borderRadius: 14, backgroundColor: C.card, borderWidth: 1, borderColor: C.amber, gap: 6 }}>
                <T v="semi" numberOfLines={1}>{focused.title || focused.short_name}</T>
                <T v="small" numberOfLines={1}>{focused.short_name}{clusterById.get(focused.cluster) ? ` · ${clusterById.get(focused.cluster)!.label}` : ''} · {focused.nights ? `${focused.nights} night${focused.nights === 1 ? '' : 's'}` : 'never tried'}</T>
                <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
                  <Btn small v="primary" testID="cloud-open" label="Open" onPress={() => router.push(`/joke/${encodeURIComponent(focused.id)}` as any)} />
                  <Btn small v="add" label="To a set" onPress={() => boards.add(focused.id, focused.title || focused.short_name)} />
                  <Btn small v="ghost" testID="cloud-done" label="Done" onPress={() => setFocus(null)} />
                </View>
              </View>
            )}
          </View>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ paddingHorizontal: 16, paddingVertical: 8, gap: 8 }} style={{ flexGrow: 0 }}>
            {m.data.clusters.map((c) => (
              <Pressable key={c.id} testID={`cloud-cluster-${c.id}`} accessibilityRole="button" accessibilityLabel={`${c.label}, ${c.size} bits. Turn it to the front.`} onPress={() => face(c)}
                style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 36, paddingHorizontal: 10, borderRadius: 18, backgroundColor: C.card }}>
                <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: slotColor(c.id % 8) }} />
                <Text numberOfLines={1} maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 13, color: C.text, maxWidth: wide ? 260 : 180 }}>{c.label}</Text>
                <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.body, fontSize: 12, color: C.mut2 }}>{c.size}</Text>
              </Pressable>
            ))}
          </ScrollView>
        </View>
      )}
    </Screen>
  );
}
