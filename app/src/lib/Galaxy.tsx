// The galaxy of bits (Bits › Map; docs/UX_2026-10.md): every bit a dot, coloured by which bits it is like (a
// cluster = the bits joined by same world · versions · tags links), shaped by status (★ kills · ● working · ○ fresh
// · – resting), sized by nights. Three zoom levels (lib/galaxy lod): overview = hulls with names, dots, no links;
// mid = links and the top labels; close = every label in view. Tap a bit to focus it (its links light up, its
// neighbours get labels, a card at the bottom); tap it again or Open for /joke/{id}; tap empty to clear; double-tap
// to zoom; hold a bit for its menu. Shuffle re-seeds the layout inside each cluster's patch (the clusters stay put).
// Drawn with react-native-svg on both platforms: one path per colour × shape for the dots (not one element per bit),
// one path per kind × weight for the links, so 1,600 bits are a few dozen elements; pan and zoom are lib/PanZoom.
// The layout runs in chunks off the render path (setTimeout) and is cached by revision + seed.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { PanResponder, PixelRatio, Platform, Pressable, ScrollView, SectionList, Text, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import Svg, { Circle, Path } from 'react-native-svg';
import { decide, useApi } from './api';
import { useBoards } from './boards';
import { afterWrite } from './feedback';
import { haptics } from './haptics';
import { GROUP_COLOR } from './jokemap';
import { PanZoom } from './PanZoom';
import type { View_ } from './PanZoom';
import { RowMenu, rowActions } from './RowMenu';
import { actionSheet } from './actionSheet';
import { getPrefs, setPrefs } from './prefs';
import type { MapGroupKey } from './simple';
import { C, F } from './theme';
import { Btn, Empty, ErrorCard, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useBottomInset, useReduceMotion, useUI, useWide } from './ui';
import { Screen } from './Screen';
import * as G from './galaxyLayout';

type AllMap = { rev: string | null; cap: number; nodes: G.GNode[]; edges: G.GEdge[]; counts: { jokes: number; nodes: number; edges: number; by_kind: Record<string, number> }; groups: { key: MapGroupKey; title: string }[] };
const CACHE = new Map<string, G.GalaxyLayout>();
const SIM_BUDGET_MS = 300;   // of simulation time, summed over the chunks
const SHUFFLE_MS = 400;
const FLY_MS = 350;
const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
const raf = (f: () => void) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(f) : setTimeout(f, 16));

export function slotColor(slot: number): string {
  return [C.cluster1, C.cluster2, C.cluster3, C.cluster4, C.cluster5, C.cluster6][slot] || (slot === G.SLOT_OTHER ? C.mut : C.dim);
}
const alpha = (hex: string, a: number) => (hex.startsWith('#') && hex.length === 7 ? `${hex}${Math.round(a * 255).toString(16).padStart(2, '0')}` : hex);

/** A legend chip: a swatch (a dot, or a dashed line for a link kind), the name, the count. */
function LegendChip({ color, dash, label, count, on, onPress, testID, a11y }: { color: string; dash?: string; label: string; count?: number; on?: boolean; onPress: () => void; testID?: string; a11y?: string }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={a11y || label} accessibilityState={{ selected: !!on }} onPress={onPress}
      style={{ minHeight: 44, paddingHorizontal: 10, borderRadius: 8, borderWidth: 1, borderColor: on ? C.amber : C.border, backgroundColor: on ? C.amberSoft : 'transparent', flexDirection: 'row', alignItems: 'center', gap: 6 }}>
      {dash === undefined ? <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: color }} />
        : <Svg width={22} height={6}><Path d="M0 3L22 3" stroke={color} strokeWidth={dash === '' ? 2 : 1.5} strokeDasharray={dash || undefined} /></Svg>}
      <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 14, color: C.text2 }}>{label}</Text>
      {count != null && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g6, fontSize: 13, color: C.mut }}>{count}</Text>}
    </Pressable>
  );
}

/** A legend row: wraps on a wide screen, scrolls sideways on the phone, one column at a large font scale (HIG). */
function LegendRow({ wide, column, children }: { wide: boolean; column: boolean; children: React.ReactNode }) {
  if (column) return <View style={{ gap: 8 }}>{children}</View>;
  if (wide) return <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>{children}</View>;
  return <ScrollView horizontal showsHorizontalScrollIndicator={false} style={{ marginHorizontal: -16, flexGrow: 0 }} contentContainerStyle={{ gap: 8, paddingHorizontal: 16 }}>{children}</ScrollView>;
}

export function Galaxy({ focusId, onCloud }: { focusId?: string; onCloud?: () => void }) {
  const wide = useWide();
  const bottom = useBottomInset();
  const reduce = useReduceMotion();
  const ui = useUI();
  const boards = useBoards();
  const native = Platform.OS !== 'web';
  const fontScale = PixelRatio.getFontScale();
  const m = useApi<AllMap>('/api/map?cap=1600', { poll: 0 });
  const [seed, setSeed] = useState(7);
  const [lay, setLay] = useState<G.GalaxyLayout | null>(null);
  const [shown, setShown] = useState<G.Placed[] | null>(null); // the positions on screen (the shuffle moves them)
  const [busy, setBusy] = useState<number | null>(null);       // layout progress 0…1 while the sim runs
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View_>({ x: 0, y: 0, scale: 1 });
  const [focus, setFocus] = useState<string | null>(null);
  const [isolated, setIsolated] = useState<string | null>(null);
  const [q, setQ] = useState('');
  const [listOpen, setListOpen] = useState(false);
  const [legendOpen, setLegendOpen] = useState(() => getPrefs().mapLegend);
  const [menuFor, setMenuFor] = useState<G.Placed | null>(null);
  const [rename, setRename] = useState<{ n: G.Placed; text: string } | null>(null);
  const viewRef = useRef(view);
  viewRef.current = view;
  const anim = useRef<{ cancel: () => void } | null>(null);
  const firstFit = useRef(false);
  const W = size.w, H = size.h;

  const kinds = useMemo(() => new Set(isolated ? [isolated] : G.ALL_KINDS), [isolated]);
  const edges = useMemo(() => (m.data?.edges || []).filter((e) => kinds.has(e.kind)), [m.data, kinds]);
  const byId = useMemo(() => new Map((shown || []).map((n) => [n.id, n])), [shown]);
  const neighbours = useMemo(() => G.neighboursOf(edges, kinds, focus), [edges, kinds, focus]);
  const clusterById = useMemo(() => new Map((lay?.clusters || []).map((c) => [c.id, c])), [lay]);

  // ---- the view's motion: a tween unless Reduce Motion
  const flyTo = useCallback((target: View_) => {
    anim.current?.cancel();
    if (reduce) return setView(target);
    const from = viewRef.current, t0 = now();
    let alive = true;
    const tick = () => {
      if (!alive) return;
      const t = Math.min(1, (now() - t0) / FLY_MS);
      setView(G.lerpView(from, target, t));
      if (t < 1) raf(tick);
    };
    anim.current = { cancel: () => { alive = false; } };
    raf(tick);
  }, [reduce]);
  const fit = useCallback((L = lay) => { if (L && W && H) flyTo(G.fitView(L, W, H)); }, [lay, W, H, flyTo]);

  // ---- the layout: in chunks off the render path, cached by rev + seed; the shuffle tweens between two layouts
  useEffect(() => {
    if (!m.data || !m.data.nodes.length) return;
    const key = `${m.data.rev}|${m.data.counts.nodes}|${seed}`;
    let cancelled = false;
    const apply = (L: G.GalaxyLayout) => {
      const prev = shown;
      setLay(L);
      if (prev && prev.length === L.nodes.length && !reduce) {
        const t0 = now();
        const tick = () => {
          if (cancelled) return;
          const t = Math.min(1, (now() - t0) / SHUFFLE_MS);
          setShown(G.interpolate(prev, L.nodes, t));
          if (t < 1) raf(tick);
        };
        raf(tick);
      } else setShown(L.nodes);
    };
    const hit = CACHE.get(key);
    if (hit) { apply(hit); return; }
    const sim = G.createSim(m.data.nodes, m.data.edges, { seed });
    let spent = 0;
    setBusy(0);
    const tick = () => {
      if (cancelled) return;
      const t0 = now();
      const done = sim.step(3);
      spent += now() - t0;
      if (!done && spent < SIM_BUDGET_MS) { setBusy(sim.progress()); setTimeout(tick, 0); return; }
      const L = sim.result();
      CACHE.set(key, L);
      setBusy(null);
      apply(L);
    };
    setTimeout(tick, 0);
    return () => { cancelled = true; };
  }, [m.data?.rev, m.data?.counts.nodes, seed]); // eslint-disable-line react-hooks/exhaustive-deps

  // the first view: the whole galaxy, or the bit this map was opened on
  useEffect(() => {
    if (!lay || !W || !H || firstFit.current) return;
    firstFit.current = true;
    const target = focusId ? lay.nodes.find((n) => n.id === focusId) : null;
    if (target) { setFocus(target.id); setView(G.centreOn(target, W, H, 1.0)); }
    else setView(G.fitView(lay, W, H));
  }, [lay, W, H, focusId]);

  // ---- what draws at this zoom
  const L = G.lod(view.scale, wide);
  const bucket = G.scaleBucket(view.scale);
  const boost = G.lod(Math.pow(2, bucket)).dotBoost;
  const rect = useMemo(() => G.worldRect(view, W, H), [view, W, H]);
  const rectKey = `${rect.x},${rect.y},${rect.w},${rect.h}`;
  const vis = useMemo(() => (shown ? G.visibleNodes(shown, rect) : []), [shown, rectKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const dots = useMemo(() => {
    const groups = new Map<string, { color: string; status: G.Status; lit: boolean; nodes: G.Placed[] }>();
    for (const n of vis) {
      if (n.id === focus) continue;
      const slot = clusterById.get(n.cluster)?.slot ?? G.SLOT_UNLINKED;
      const st = G.statusOf(n);
      const lit = !focus || neighbours.has(n.id);
      const k = `${slot}|${st}|${lit ? 1 : 0}`;
      let g = groups.get(k);
      if (!g) groups.set(k, (g = { color: slotColor(slot), status: st, lit, nodes: [] }));
      g.nodes.push(n);
    }
    const sw = 2 / Math.pow(2, bucket);
    const out: React.ReactNode[] = [];
    for (const [k, g] of groups) {
      const r = (n: G.Placed) => G.nodeRadius(n) * boost;
      const op = g.lit ? 1 : 0.3;
      if (g.status === 'fresh') out.push(<Path key={k} d={G.dotsPath(g.nodes, r)} fill="none" stroke={g.color} strokeWidth={sw} opacity={op} />);
      else if (g.status === 'resting') out.push(<Path key={k} d={G.dotsPath(g.nodes, (n) => Math.max(1, r(n) - 1))} fill={g.color} opacity={0.55 * op} />);
      else {
        out.push(<Path key={k} d={G.dotsPath(g.nodes, r)} fill={g.color} opacity={op} />);
        if (g.status === 'kills') out.push(<Path key={`${k}|core`} d={G.dotsPath(g.nodes, () => sw)} fill="#FFFFFF" opacity={op} />);
      }
    }
    return out;
  }, [vis, focus, neighbours, clusterById, bucket, boost]);
  const edgeEls = useMemo(() => {
    const list = G.edgesToDraw(edges, kinds, focus, view.scale, byId, rect);
    const groups = new Map<string, { kind: string; w: number; d: G.GEdge[] }>();
    for (const e of list) {
      const wb = Math.round(Math.max(0, Math.min(1, e.w)) * 4) / 4;
      const k = `${e.kind}|${wb}`;
      let g = groups.get(k);
      if (!g) groups.set(k, (g = { kind: e.kind, w: wb, d: [] }));
      g.d.push(e);
    }
    const inv = 1 / Math.pow(2, bucket);
    return [...groups.entries()].map(([k, g]) => {
      const s = G.edgeStroke({ a: '', b: '', kind: g.kind, w: g.w });
      return <Path key={k} d={G.edgesPath(g.d, byId)} stroke={GROUP_COLOR[g.kind as MapGroupKey] || C.mut} strokeWidth={(focus ? 2.5 : s.width) * inv} strokeOpacity={focus ? 0.9 : s.alpha} strokeDasharray={G.EDGE_DASH[g.kind] ? G.EDGE_DASH[g.kind]!.split(',').map((x) => Number(x) * inv).join(',') : undefined} fill="none" />;
    });
  }, [edges, kinds, focus, view.scale, byId, rectKey, bucket]); // eslint-disable-line react-hooks/exhaustive-deps
  const hulls = useMemo(() => (lay && L.clusterLabels && W ? G.hullLabels(lay.clusters, view, W, H) : []), [lay, L.clusterLabels, view, W, H]);
  const labels = useMemo(() => (W ? G.cullLabels(vis, view, W, H, { focus, neighbours, maxLabels: L.maxLabels, dotBoost: boost, taken: hulls.map((x) => x.box) }) : []), [vis, view, W, H, focus, neighbours, L.maxLabels, boost, hulls]);
  const whys = useMemo(() => (focus && L.level === 'close' ? G.whyLabels(G.edgesToDraw(edges, kinds, focus, view.scale, byId, rect), byId, view, labels.map((x) => x.box)) : []), [edges, kinds, focus, L.level, view, byId, rectKey, labels]); // eslint-disable-line react-hooks/exhaustive-deps
  const focused = focus ? byId.get(focus) || null : null;
  const focusCluster = focused ? clusterById.get(focused.cluster) : null;
  const inv = 1 / view.scale;

  // ---- taps on the canvas: the nearest dot within 18 screen px, or empty space; a double tap zooms
  const lastTap = useRef<{ t: number; x: number; y: number } | null>(null);
  const worldAt = (e: any): { x: number; y: number } => {
    const ne = e.nativeEvent || {};
    if (!native && typeof ne.offsetX === 'number') return { x: ne.offsetX, y: ne.offsetY }; // the DOM gives local (world) coordinates
    if (native) return { x: ne.locationX, y: ne.locationY };
    return { x: (ne.locationX || 0) / viewRef.current.scale, y: (ne.locationY || 0) / viewRef.current.scale };
  };
  const select = useCallback((n: G.Placed) => {
    haptics.selection();
    setFocus(n.id);
  }, []);
  const onCanvasPress = (e: any) => {
    if (!shown) return;
    const p = worldAt(e);
    const v = viewRef.current;
    const prev = lastTap.current, t = now();
    lastTap.current = { t, x: p.x, y: p.y };
    if (prev && t - prev.t < 300 && Math.hypot(prev.x - p.x, prev.y - p.y) * v.scale < 30) {
      lastTap.current = null;
      if (v.scale >= G.ZOOM_CLOSE) return fit();
      return flyTo(G.zoomAtPoint(v, 2, v.x + p.x * v.scale, v.y + p.y * v.scale));
    }
    const hit = G.nearest(vis, p.x, p.y, 18 / v.scale);
    if (!hit) return setFocus(null);
    if (hit.id === focus) return router.push(`/joke/${encodeURIComponent(hit.id)}` as any);
    select(hit);
  };
  const menuItems = useCallback((n: G.GNode): MenuItem[] => [
    { label: 'Open', icon: 'arrow.up.right.square', onPress: () => router.push(`/joke/${encodeURIComponent(n.id)}` as any) },
    { label: 'To a set…', icon: 'text.badge.plus', onPress: () => (boards.current ? afterWrite(() => boards.add(n.id, n.title || n.short_name), (ok) => ok, haptics) : boards.pick(n.id, n.title || n.short_name)) },
    { label: 'Merge with…', icon: 'arrow.triangle.merge', onPress: () => router.push('/merge' as any) },
    { label: 'Rename short name…', icon: 'pencil', onPress: () => setRename({ n: n as G.Placed, text: n.short_name }) },
  ], [boards]);
  const openMenu = (n: G.Placed) => {
    const items = menuItems(n);
    if (Platform.OS === 'ios') actionSheet({ title: n.title || n.short_name, options: [...items.map((it) => ({ label: it.label, onPress: it.onPress })), { label: 'Cancel', cancel: true }] });
    else setMenuFor(n);
  };
  const onCanvasLongPress = (e: any) => {
    if (!shown) return;
    const p = worldAt(e);
    const hit = G.nearest(vis, p.x, p.y, 18 / viewRef.current.scale);
    if (hit) { setFocus(hit.id); openMenu(hit); }
  };
  const flyToNode = (n: { x: number; y: number }) => { if (W && H) flyTo(G.centreOn(n, W, H, Math.max(G.ZOOM_CLOSE, Math.min(1.6, viewRef.current.scale)))); };
  const pickResult = (n: G.GNode) => {
    const p = byId.get(n.id);
    setQ('');
    if (p) { select(p); flyToNode(p); }
  };
  const shuffle = () => { setSeed((s) => s + 1); };
  const centre = () => { if (focused && W && H) flyTo(G.centreOn(focused, W, H, Math.max(1, viewRef.current.scale))); else fit(); };
  const card = useRef(PanResponder.create({
    onMoveShouldSetPanResponder: (_, g) => Math.abs(g.dy) > 12 && Math.abs(g.dy) > Math.abs(g.dx),
    onPanResponderRelease: (_, g) => { if (g.dy > 40) setFocus(null); },
  })).current;
  const saveRename = async () => {
    if (!rename) return;
    const text = rename.text.trim().slice(0, 24);
    try {
      await decide({ target: 'node', action: 'edit', id: rename.n.id, after: { short_name: text || null } });
      setRename(null);
      haptics.success();
      ui.snack(text ? `Short name: ${text}` : 'Short name reset');
      m.reload();
    } catch (e: any) {
      ui.snack(`Couldn't rename: ${e.message}`);
    }
  };
  const results = useMemo(() => (m.data ? G.searchNodes(m.data.nodes, q, 8) : []), [m.data, q]);
  const total = m.data?.counts.nodes || 0;
  const groupsN = lay ? lay.clusters.filter((c) => !c.loose).length : 0;
  const isolatedGroup = isolated ? m.data?.groups.find((g) => g.key === isolated) || null : null;
  const counts = m.data ? G.countsLine(total, edges.length, groupsN, isolatedGroup) : '';
  const oneColumn = fontScale >= 1.3;

  const toolbar = native ? (
    <Stack.Toolbar placement="right">
      <Stack.Toolbar.Button icon="shuffle" accessibilityLabel="Shuffle the layout" onPress={shuffle}>Shuffle</Stack.Toolbar.Button>
      <Stack.Toolbar.Button icon="scope" accessibilityLabel="Centre on the selected bit" onPress={centre}>Centre</Stack.Toolbar.Button>
      <Stack.Toolbar.Button icon="list.bullet" accessibilityLabel={listOpen ? 'Back to the map' : 'Read the map as a list'} onPress={() => setListOpen((v) => !v)}>List</Stack.Toolbar.Button>
      {onCloud ? <Stack.Toolbar.Button icon="cloud" accessibilityLabel="The cloud: bits placed by meaning" onPress={onCloud}>Cloud</Stack.Toolbar.Button> : null}
    </Stack.Toolbar>
  ) : null;
  const webRight = !native ? (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, paddingRight: 8 }}>
      <TextBtn testID="galaxy-shuffle" label="Shuffle" onPress={shuffle} />
      <TextBtn testID="galaxy-centre" label="Centre" onPress={centre} />
      <TextBtn testID="galaxy-list" label={listOpen ? 'Map' : 'List'} onPress={() => setListOpen((v) => !v)} />
      {onCloud && <TextBtn testID="galaxy-cloud" label="Cloud" onPress={onCloud} />}
    </View>
  ) : undefined;

  // the frame's last row: it clears the home indicator and the floating tab bar, and so does the canvas above it with
  // its card and zoom buttons (build 18 drew them at a fixed 12 pt, under the bar on an iPhone 16)
  const legend = m.data && lay ? (
    <View style={{ paddingHorizontal: 16, gap: 6, paddingBottom: bottom }}>
      <Pressable testID="galaxy-legend-toggle" accessibilityRole="button" accessibilityState={{ expanded: legendOpen }} onPress={() => { setLegendOpen((v) => { setPrefs({ mapLegend: !v }); return !v; }); }} style={{ minHeight: 44, justifyContent: 'center' }}>
        <T v="small" style={{ color: C.amber }}>Legend {legendOpen ? '⌃' : '⌄'}</T>
      </Pressable>
      {legendOpen && (
        <View style={{ gap: 8, paddingBottom: 8 }}>
          <LegendRow wide={wide} column={oneColumn}>
            {lay.clusters.map((c) => (
              <LegendChip key={c.id} testID={`galaxy-cluster-${c.id}`} color={slotColor(c.slot)} label={c.label} count={c.size} a11y={`${c.label}, ${c.size} bits. Go there.`}
                onPress={() => { if (W && H) flyTo(G.centreOn({ x: c.cx, y: c.cy }, W, H, Math.max(G.MIN_SCALE, Math.min(1.2, Math.min(W, H) / (c.r * 2.4))))); }} />
            ))}
          </LegendRow>
          <LegendRow wide={wide} column={oneColumn}>
            {m.data.groups.map((g) => (
              <LegendChip key={g.key} testID={`galaxy-kind-${g.key}`} color={GROUP_COLOR[g.key]} dash={G.EDGE_DASH[g.key] || ''} label={g.title} count={m.data!.counts.by_kind[g.key] || 0} on={isolated === g.key}
                a11y={isolated === g.key ? `${g.title}, showing only these links. Show all.` : `${g.title}. Show only these links.`} onPress={() => setIsolated((k) => (k === g.key ? null : g.key))} />
            ))}
            {isolated && <LegendChip testID="galaxy-kind-all" color={C.mut} dash="" label="All" onPress={() => setIsolated(null)} />}
          </LegendRow>
        </View>
      )}
    </View>
  ) : null;

  const list = lay && m.data ? (
    <SectionList
      testID="galaxy-list-view"
      style={{ flex: 1 }}
      contentContainerStyle={{ paddingHorizontal: 16, paddingBottom: 120 }}
      sections={lay.clusters.map((c) => ({ key: String(c.id), c, data: lay.nodes.filter((n) => n.cluster === c.id).sort(G.labelOrder) }))}
      keyExtractor={(n) => n.id}
      renderSectionHeader={({ section }) => (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingVertical: 10, backgroundColor: C.bg }}>
          <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: slotColor((section as any).c.slot) }} />
          <T v="eb" style={{ flex: 1 }}>{(section as any).c.label} · {(section as any).c.size}</T>
        </View>
      )}
      renderItem={({ item: n }) => {
        const st = G.statusOf(n);
        const links = edges.reduce((k, e) => k + (e.a === n.id || e.b === n.id ? 1 : 0), 0);
        return (
          <Pressable accessibilityRole="button" accessibilityLabel={`${n.short_name}, ${G.STATUS_WORD[st]}, ${n.nights} nights, ${links} links`} onPress={() => router.push(`/joke/${encodeURIComponent(n.id)}` as any)}
            style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 6 }}>
            <Text style={{ fontFamily: F.semi, fontSize: 15, color: C.text, width: 18, textAlign: 'center' }}>{G.STATUS_GLYPH[st]}</Text>
            <T style={{ flex: 1 }} numberOfLines={1}>{n.short_name}</T>
            <T v="small">{n.nights} night{n.nights === 1 ? '' : 's'} · {links} link{links === 1 ? '' : 's'}</T>
          </Pressable>
        );
      }}
    />
  ) : null;

  const canvas = lay && shown ? (
    <View style={{ flex: 1 }} onLayout={(e) => { const { width, height } = e.nativeEvent.layout; if (width !== size.w || height !== size.h) setSize({ w: width, h: height }); }}>
      {W > 0 && H > 0 && (
        <PanZoom view={view} onView={setView} width={W} height={H} minScale={Math.min(G.MIN_SCALE, G.fitView(lay, W, H).scale)} maxScale={G.MAX_SCALE} testID="galaxy-canvas">
          <Svg width={lay.width} height={lay.height} style={{ position: 'absolute', left: 0, top: 0 }} pointerEvents="none">
            {lay.clusters.filter((c) => !c.loose).map((c) => <Circle key={c.id} cx={c.cx} cy={c.cy} r={c.r} fill={alpha(slotColor(c.slot), 0.12)} stroke={alpha(slotColor(c.slot), 0.25)} strokeWidth={inv} />)}
            {edgeEls}
            {dots}
            {focused && <Circle cx={focused.x} cy={focused.y} r={G.FOCUS_RADIUS * boost} fill={slotColor(focusCluster?.slot ?? G.SLOT_UNLINKED)} stroke={C.amber} strokeWidth={2.5 * inv} />}
          </Svg>
          <Pressable testID="galaxy-hit" accessibilityRole="image" accessibilityLabel={`Map of ${total} bits in ${groupsN} groups. Use List to read it.`}
            onPress={onCanvasPress} onLongPress={onCanvasLongPress} delayLongPress={400} style={{ position: 'absolute', left: 0, top: 0, width: lay.width, height: lay.height }} />
          {hulls.map(({ c }) => (
            <View key={c.id} pointerEvents="none" style={{ position: 'absolute', left: c.cx - 130 * inv, top: c.cy - c.r - 22 * inv, width: 260, alignItems: 'center', transform: [{ scale: inv }], transformOrigin: 'top left' } as any}>
              <Text maxFontSizeMultiplier={1.2} numberOfLines={1} style={{ fontFamily: F.g6, fontSize: G.HULL_FONT, color: slotColor(c.slot), textAlign: 'center', textShadowColor: C.bg, textShadowRadius: 4 }}>{c.label} · {c.size}</Text>
            </View>
          ))}
          {whys.map(({ e, mx, my }, i) => (
            <View key={i} pointerEvents="none" style={{ position: 'absolute', left: mx - 90 * inv, top: my - 8 * inv, width: 180, alignItems: 'center', transform: [{ scale: inv }], transformOrigin: 'top left' } as any}>
              <Text maxFontSizeMultiplier={1.2} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 11, color: C.mut, backgroundColor: C.bg, paddingHorizontal: 4, borderRadius: 4, textAlign: 'center' }}>{e.why}</Text>
            </View>
          ))}
          {labels.map((n) => {
            const on = n.id === focus, near = neighbours.has(n.id);
            const col = slotColor(clusterById.get(n.cluster)?.slot ?? G.SLOT_UNLINKED);
            const cl = clusterById.get(n.cluster)?.label || 'Unlinked';
            const st = G.statusOf(n);
            const items = menuItems(n);
            return (
              <RowMenu key={n.id} href={`/joke/${encodeURIComponent(n.id)}`} items={items}>
                <Pressable testID={`galaxy-node-${n.id}`} accessibilityRole="button" accessibilityLabel={`${n.short_name}, ${n.nights} night${n.nights === 1 ? '' : 's'}. ${on ? 'Open' : 'Select'}`} accessibilityValue={{ text: `in ${cl}, ${G.STATUS_WORD[st].toLowerCase()}` }} {...rowActions(items)}
                  onPress={() => (on ? router.push(`/joke/${encodeURIComponent(n.id)}` as any) : select(n))} onLongPress={() => openMenu(n)} delayLongPress={400}
                  style={{ position: 'absolute', left: n.x + (n.box.x - n.sx) * inv, top: n.y + (n.box.y - n.sy) * inv, width: n.box.w, height: n.box.h, transform: [{ scale: inv }], transformOrigin: 'top left', borderRadius: 12, borderWidth: on ? 2 : 1, borderColor: on ? C.amber : col, backgroundColor: C.surface, paddingHorizontal: 8, justifyContent: 'center', opacity: focus && !on && !near ? 0.3 : 1 } as any}>
                  <Text numberOfLines={1} maxFontSizeMultiplier={1.2} style={{ fontFamily: on ? F.semi : F.medium, fontSize: G.LABEL_FONT, color: C.text }}>{n.short_name}</Text>
                </Pressable>
              </RowMenu>
            );
          })}
        </PanZoom>
      )}
      {!native && q.length > 0 && results.length > 0 && (
        <View testID="galaxy-results" style={{ position: 'absolute', left: 12, right: 12, top: 8, borderRadius: 12, backgroundColor: C.card, borderWidth: 1, borderColor: C.border, zIndex: 4 }}>
          {results.map((n) => <Pressable key={n.id} testID={`galaxy-result-${n.id}`} accessibilityRole="button" onPress={() => pickResult(n)} style={{ minHeight: 44, paddingHorizontal: 12, justifyContent: 'center' }}><T numberOfLines={1}>{n.short_name}<Text style={{ color: C.mut }}>{n.title && n.title !== n.short_name ? ` · ${n.title}` : ''}</Text></T></Pressable>)}
        </View>
      )}
      {focused && (
        <View {...card.panHandlers} testID="galaxy-card" style={{ position: 'absolute', left: 12, right: 72, bottom: 12, maxWidth: 520, padding: 12, borderRadius: 14, backgroundColor: C.card, borderWidth: 1, borderColor: C.amber, gap: 6 }}>
          <RowMenu href={`/joke/${encodeURIComponent(focused.id)}`} items={menuItems(focused)}>
            <View style={{ gap: 4 }}>
              <T v="semi" numberOfLines={1}>{focused.title || focused.short_name}</T>
              <T v="small" numberOfLines={1}>{focused.short_name}{focusCluster && !focusCluster.loose ? ` · ${focusCluster.label}` : ''}</T>
              <T v="small">{G.STATUS_GLYPH[G.statusOf(focused)]} {G.STATUS_WORD[G.statusOf(focused)]} · {focused.nights ? `${focused.nights} night${focused.nights === 1 ? '' : 's'}` : 'never tried'} · {neighbours.size ? `${neighbours.size} link${neighbours.size === 1 ? '' : 's'}` : 'nothing linked yet'}</T>
            </View>
          </RowMenu>
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
            <Btn small v="primary" testID="galaxy-open" label="Open" onPress={() => router.push(`/joke/${encodeURIComponent(focused.id)}` as any)} />
            <Btn small v="add" label="To a set" onPress={() => menuItems(focused)[1].onPress()} />
            <Btn small label="Merge" onPress={() => router.push('/merge' as any)} />
            <Btn small v="ghost" testID="galaxy-done" label="Done" onPress={() => setFocus(null)} />
          </View>
        </View>
      )}
    </View>
  ) : null;

  return (
    <Screen title="Map" back backLabel="Bits" testID="galaxy" scroll={false} right={webRight}
      search={native ? { placeholder: 'Find a bit', onChangeText: (e: any) => setQ(e.nativeEvent.text), hideWhenScrolling: false } as any : undefined}>
      {toolbar}
      {m.rawError && !m.data ? <View style={{ padding: 16 }}><ErrorCard error={m.rawError} onRetry={m.reload} /></View> : null}
      {!m.data && !m.rawError && <View style={{ padding: 16 }}><Skeleton lines={5} height={18} testID="galaxy-skeleton" /></View>}
      {m.data && !m.data.nodes.length && <Empty text="No bits to map yet." />}
      {m.data && m.data.nodes.length > 0 && (
        <View style={{ flex: 1 }}>
          <View style={{ paddingHorizontal: 16, paddingTop: 8, gap: 6 }}>
            <T v="small" testID="galaxy-counts">{busy != null && !lay ? `Laying out ${total} bits… ${Math.round(busy * 100)}%` : counts}{lay && !focus && !listOpen ? ' · tap a bit to bring its links in' : ''}</T>
            {focusId && lay && !lay.nodes.some((n) => n.id === focusId) && <T v="small" testID="galaxy-notonmap">This bit isn't on the map: only the {lay.nodes.length} most performed are.</T>}
            {!native && (
              <TextInput testID="galaxy-find" value={q} onChangeText={setQ} placeholder="Find a bit" placeholderTextColor={C.dim} accessibilityLabel="Find a bit" autoCorrect={false}
                style={{ minHeight: 44, borderRadius: 10, borderWidth: 1, borderColor: C.border, backgroundColor: C.surface, paddingHorizontal: 12, fontFamily: F.body, fontSize: 15, color: C.text }} />
            )}
          </View>
          {native && q.length > 0 && results.length > 0 && (
            <View testID="galaxy-results" style={{ marginHorizontal: 16, borderRadius: 12, backgroundColor: C.card, borderWidth: 1, borderColor: C.border }}>
              {results.map((n) => <Pressable key={n.id} accessibilityRole="button" onPress={() => pickResult(n)} style={{ minHeight: 44, paddingHorizontal: 12, justifyContent: 'center' }}><T numberOfLines={1}>{n.short_name}</T></Pressable>)}
            </View>
          )}
          {!lay && busy != null && <View style={{ padding: 16 }}><Skeleton lines={4} height={18} /></View>}
          {listOpen ? list : canvas}
          {!listOpen && legend}
        </View>
      )}
      <MenuSheet visible={!!menuFor} title={menuFor?.title || menuFor?.short_name} items={menuFor ? menuItems(menuFor).map((it) => ({ ...it, onPress: () => { setMenuFor(null); it.onPress(); } })) : []} onClose={() => setMenuFor(null)} testID="galaxy-menu" />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename short name" testID="galaxy-rename">
        <SheetHead title="Short name" onCancel={() => setRename(null)} onDone={saveRename} doneLabel="Save" />
        <TextInput value={rename?.text || ''} onChangeText={(t) => setRename((r) => (r ? { ...r, text: t } : r))} maxLength={24} autoFocus placeholder="What you'd write on the set list" placeholderTextColor={C.dim}
          style={{ minHeight: 44, borderRadius: 10, borderWidth: 1, borderColor: C.border, backgroundColor: C.surface, paddingHorizontal: 12, fontFamily: F.body, fontSize: 17, color: C.text }} />
      </Sheet>
    </Screen>
  );
}
