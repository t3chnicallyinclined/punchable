// Bits › Map (docs/UX_2026-10.md): a real bird's-eye view of ALL his material. One request, GET /api/map?cap=…
// (names and links only, cached by rev on the server and by path on this device); the layout is lib/allmap
// (clusters = the per-joke map's groups: next · tags · same world · callbacks · versions). Level of detail: the top
// N bits (nights, laughs) are laid out first; tap a bit to light its links and bring its neighbours in; tap it
// again (or Open) for the bit page. Zoom/pan: lib/PanZoom (pointer + wheel on the web, pan + pinch on the phone).
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Platform, Pressable, Text, View, useWindowDimensions } from 'react-native';
import { Stack, router } from 'expo-router';
import Svg, { Circle, Line } from 'react-native-svg';
import { useApi } from '@/lib/api';
import { Layout, MapEdge, MapNode, Placed, layout, placeNear, topNodes, visible } from '@/lib/allmap';
import { haptics } from '@/lib/haptics';
import { GROUP_COLOR } from '@/lib/jokemap';
import { PanZoom, View_, zoomAt } from '@/lib/PanZoom';
import { MapGroupKey } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { Btn, Chip, Chips, Empty, ErrorCard, SafeBottom, Skeleton, T, useReduceMotion, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type AllMap = { rev: string | null; cap: number; nodes: MapNode[]; edges: MapEdge[]; counts: { jokes: number; nodes: number; edges: number; by_kind: Record<string, number> }; groups: { key: MapGroupKey; title: string }[] };
const FIRST_N = 300;       // laid out at once; the rest come in as he expands
const LABEL_MIN_SCALE = 0.6; // below this only dots draw (a readable map, not a word cloud)
type Status = 'kills' | 'working' | 'fresh' | 'resting';
const STATUSES: Status[] = ['kills', 'working', 'fresh', 'resting'];
// read at render so the Light / Dark switch reaches the dots (theme.ts tokens with light variants)
const dotOf = (st: Status) => ({ kills: C.statusKills, working: C.statusWorking, fresh: C.statusFresh, resting: C.statusResting }[st]);
function status(n: MapNode): Status {
  if (n.stage === 'retired') return 'resting';
  if (n.nights >= 2 && (n.heat || 0) >= 1.3) return 'kills';
  if (n.nights > 0 || n.stage === 'working') return 'working';
  return 'fresh';
}

export default function AllBitsMap() {
  const wide = useWide();
  const { width, height } = useWindowDimensions();
  const reduce = useReduceMotion();
  const m = useApi<AllMap>('/api/map?cap=1600', { poll: 0 });
  const [placed, setPlaced] = useState<Placed[] | null>(null);
  const [canvas, setCanvas] = useState<Layout | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [kinds, setKinds] = useState<Set<string>>(new Set(['next', 'tags', 'same_world', 'callbacks', 'versions']));
  const W = wide ? width - 232 - 64 : width;
  const H = Math.max(360, height - 200);
  const [view, setView] = useState<View_>({ x: 0, y: 0, scale: 1 });
  const edges = useMemo(() => (m.data?.edges || []).filter((e) => kinds.has(e.kind)), [m.data, kinds]);

  // the first draw: the top N, laid out once per map revision, fitted into the viewport
  useEffect(() => {
    if (!m.data) return;
    const top = topNodes(m.data.nodes, FIRST_N);
    const L = layout(top, m.data.edges);
    setCanvas(L);
    setPlaced(L.nodes);
    const s = Math.min(1, Math.max(0.25, Math.min(W / (L.width + 80), H / (L.height + 80))));
    setView({ scale: s, x: (W - L.width * s) / 2, y: (H - L.height * s) / 2 });
    setSel(null);
  }, [m.data?.rev, m.data?.counts.nodes]); // eslint-disable-line react-hooks/exhaustive-deps

  const byId = useMemo(() => new Map((placed || []).map((n) => [n.id, n])), [placed]);
  const shown = useMemo(() => (placed ? visible(placed, { x: view.x, y: view.y, w: W, h: H, scale: view.scale }) : []), [placed, view, W, H]);
  const shownIds = useMemo(() => new Set(shown.map((n) => n.id)), [shown]);
  const lines = useMemo(() => edges.filter((e) => byId.has(e.a) && byId.has(e.b) && (shownIds.has(e.a) || shownIds.has(e.b))), [edges, byId, shownIds]);
  const neighbours = useMemo(() => {
    if (!sel) return new Set<string>();
    const s = new Set<string>();
    for (const e of edges) if (e.a === sel) s.add(e.b); else if (e.b === sel) s.add(e.a);
    return s;
  }, [sel, edges]);
  const labels = view.scale >= LABEL_MIN_SCALE;

  const tap = useCallback((n: Placed) => {
    if (sel === n.id) return router.push(`/joke/${encodeURIComponent(n.id)}` as any);
    haptics.selection();
    setSel(n.id);
    if (m.data && placed) setPlaced(placeNear(placed, m.data.nodes, m.data.edges, n.id));
  }, [sel, m.data, placed]);
  const selected = sel ? byId.get(sel) || null : null;
  const fit = useCallback(() => {
    if (canvas) setView(zoomAt({ x: 0, y: 0, scale: 1 }, Math.min(1, Math.min(W / (canvas.width + 80), H / (canvas.height + 80))), 0, 0));
  }, [canvas, W, H]);
  const native = Platform.OS !== 'web';
  const total = m.data?.counts.jokes || 0;
  const offscreen = m.data && placed ? m.data.nodes.length - placed.length : 0;

  return (
    <Screen title="Map" back backLabel="Bits" testID="allmap" scroll={false}>
      {m.rawError && !m.data ? <View style={{ padding: 16 }}><ErrorCard error={m.rawError} onRetry={m.reload} /></View> : null}
      {!m.data && !m.rawError && <View style={{ padding: 16 }}><Skeleton lines={5} height={18} testID="allmap-skeleton" /></View>}
      {m.data && !m.data.nodes.length && <Empty text="No bits to map yet." />}
      {native && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="arrow.down.left.and.arrow.up.right" accessibilityLabel="Fit the map to the screen" onPress={fit}>Fit</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {m.data && placed && canvas && (
        <View style={{ flex: 1 }}>
          <View style={{ paddingHorizontal: 16, paddingVertical: 8, gap: 8 }}>
            <T v="small" testID="allmap-counts">{placed.length} of {total} bits drawn{offscreen > 0 ? ` · tap a bit to bring its links in` : ''} · {lines.length} links</T>
            <Chips testID="allmap-kinds">
              {m.data.groups.map((g) => <Chip key={g.key} testID={`allmap-kind-${g.key}`} label={`${g.title} · ${m.data!.counts.by_kind[g.key] || 0}`} on={kinds.has(g.key)} onPress={() => setKinds((k) => { const n = new Set(k); if (n.has(g.key)) n.delete(g.key); else n.add(g.key); return n; })} />)}
            </Chips>
          </View>
          <PanZoom view={view} onView={setView} width={W} height={H} testID="allmap-canvas">
            <Svg width={canvas.width + 400} height={canvas.height + 400} style={{ position: 'absolute', left: 0, top: 0 }} pointerEvents="none">
              {lines.map((e, i) => {
                const a = byId.get(e.a)!, b = byId.get(e.b)!;
                const lit = sel && (e.a === sel || e.b === sel);
                return <Line key={i} x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke={GROUP_COLOR[e.kind as MapGroupKey] || C.mut} strokeOpacity={sel ? (lit ? 0.9 : 0.12) : 0.35 + 0.35 * e.w} strokeWidth={lit ? 2.5 : 1.2} />;
              })}
              {!labels && shown.map((n) => <Circle key={n.id} cx={n.x} cy={n.y} r={n.id === sel ? 9 : 4 + Math.min(4, n.nights)} fill={dotOf(status(n))} opacity={sel && n.id !== sel && !neighbours.has(n.id) ? 0.35 : 1} />)}
            </Svg>
            {labels && shown.map((n) => {
              const on = n.id === sel, near = neighbours.has(n.id);
              return (
                <Pressable key={n.id} testID={`allmap-node-${n.id}`} accessibilityRole="button" accessibilityLabel={`${n.short_name}, ${n.nights} night${n.nights === 1 ? '' : 's'}. ${on ? 'Open' : 'Select'}`} onPress={() => tap(n)}
                  style={{ position: 'absolute', left: n.x - 60, top: n.y - 16, width: 120, minHeight: 32, borderRadius: 16, borderWidth: on ? 2 : 1, borderColor: on ? C.amber : near ? C.text : C.border, backgroundColor: C.surface, paddingHorizontal: 8, flexDirection: 'row', alignItems: 'center', gap: 6, opacity: sel && !on && !near ? 0.45 : 1 }}>
                  <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: dotOf(status(n)) }} />
                  <Text numberOfLines={1} maxFontSizeMultiplier={1.2} style={{ flex: 1, fontFamily: on ? F.semi : F.medium, fontSize: 12, color: C.text }}>{n.short_name}</Text>
                </Pressable>
              );
            })}
          </PanZoom>
          {selected && (
            <View testID="allmap-selected" style={{ position: 'absolute', left: 12, right: 72, bottom: 12, padding: 12, borderRadius: 14, backgroundColor: C.card, borderWidth: 1, borderColor: C.amber, gap: 6 }}>
              <T v="semi" numberOfLines={1}>{selected.title || selected.short_name}</T>
              <T v="small">{selected.nights ? `${selected.nights} night${selected.nights === 1 ? '' : 's'}` : 'never tried'}{neighbours.size ? ` · ${neighbours.size} link${neighbours.size === 1 ? '' : 's'}` : ' · nothing linked yet'}</T>
              <View style={{ flexDirection: 'row', gap: 8 }}>
                <Btn small v="primary" testID="allmap-open" label="Open" onPress={() => router.push(`/joke/${encodeURIComponent(selected.id)}` as any)} />
                <Btn small v="ghost" label="Centre" onPress={() => setView((v) => ({ ...v, x: W / 2 - selected.x * v.scale, y: H / 2 - selected.y * v.scale }))} />
                <Btn small v="ghost" label="Done" onPress={() => setSel(null)} />
              </View>
            </View>
          )}
          {!reduce && null}
          <SafeBottom style={{ flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 12, paddingHorizontal: 16, paddingVertical: 8 }}>
            {STATUSES.map((k) => <View key={k} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}><View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: dotOf(k) }} /><T v="small">{k[0].toUpperCase() + k.slice(1)}</T></View>)}
            {!native && <Pressable accessibilityRole="button" accessibilityLabel="Fit the map to the screen" onPress={fit} style={{ minHeight: 44, minWidth: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Fit</T></Pressable>}
          </SafeBottom>
        </View>
      )}
    </Screen>
  );
}
