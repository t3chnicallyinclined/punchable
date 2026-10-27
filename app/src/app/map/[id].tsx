// The joke map (SIMPLE.md): opening any joke puts it in the middle with its connections grouped: Could come next ·
// Could tag it · Same world · Callbacks · Older versions · Ideas (the AI's, folded). Tapping a connected joke
// re-centers on it (walk the map; "‹ Back to …" walks back). Phone: the joke, then one sideways row per group.
// Wide: a Map toggle draws the bird's-eye view. "Start a set from here" makes a board and opens the set builder.
// The old workspace (stage, numbers, versions, brainstorm) is one tap down at /bit/<id>.
import React, { useEffect, useState } from 'react';
import { Pressable, ScrollView, Text, View, useWindowDimensions } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import Svg, { Circle, Line } from 'react-native-svg';
import { post, useApi } from '@/lib/api';
import { creative } from '@/lib/creative';
import { useBoards } from '@/lib/boards';
import { BitCard, JokeMap, MapGroupKey, draft, monthYear, quoteText } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { AddBtn, Btn, Card, Empty, ErrorCard, Screen, Seg, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { MergeHint } from '@/lib/MergeHint';

const GROUP_COLOR: Record<MapGroupKey, string> = { next: C.amber, tags: C.green, same_world: C.blue, callbacks: C.redSoft, versions: C.mut };
const CARD_W = 248;

function MapCard({ b, g, i, onOpen, onAdd }: { b: BitCard; g: MapGroupKey; i: number; onOpen: () => void; onAdd: () => void }) {
  return (
    <View testID={`map-${g}-${i}`} style={[s.inner, { width: CARD_W, gap: 6, borderTopWidth: 3, borderTopColor: GROUP_COLOR[g] }]}>
      <Pressable testID={`map-${g}-${i}-open`} accessibilityRole="button" accessibilityLabel={`${b.title}. ${g === 'versions' ? 'Open this version' : 'Put it in the middle'}`} onPress={onOpen} style={{ gap: 4, minHeight: 44 }}>
        <T v="semi" numberOfLines={1}>{b.title}{b.date ? <Text style={{ fontFamily: F.body, color: C.mut }}> · {monthYear(b.date)}</Text> : null}</T>
        {!!b.quote && <T numberOfLines={3} style={{ fontSize: 14, lineHeight: 20, color: C.text2 }}>{quoteText(b)}</T>}
      </Pressable>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        <View style={{ flex: 1, minWidth: 0 }}>
          {!!b.why && <T v="small" numberOfLines={2}>{b.mine ? 'Yours · ' : ''}{b.why}</T>}
          {!!b.stage_line && <T v="small" numberOfLines={1} style={{ color: b.proven ? C.green : C.mut }}>{b.stage_line}</T>}
        </View>
        {g !== 'versions' && <AddBtn testID={`map-${g}-${i}-add`} onPress={onAdd} label={`Add ${b.title} to a board`} />}
      </View>
    </View>
  );
}

/** The bird's-eye view: the joke in the middle, each group in its own slice of the circle. */
function Radial({ map, onOpen, width }: { map: JokeMap; onOpen: (b: BitCard, g: MapGroupKey) => void; width: number }) {
  const groups = map.groups.filter((g) => g.items.length);
  // an ellipse that fills the width; each group gets its own slice, its bits alternate between two rings
  const H = 640, W = width, cx = W / 2, cy = H / 2 - 10, rx = W / 2 - 80, ry = H / 2 - 50;
  const nodes: { b: BitCard; g: MapGroupKey; x: number; y: number }[] = [];
  const slice = (2 * Math.PI) / Math.max(1, groups.length);
  groups.forEach((g, gi) => {
    const its = g.items.slice(0, 5);
    its.forEach((b, k) => {
      const a = -Math.PI / 2 + gi * slice + (slice * (k + 0.5)) / its.length;
      const r = k % 2 ? 0.58 : 1;
      nodes.push({ b, g: g.key, x: cx + rx * r * Math.cos(a), y: cy + ry * r * Math.sin(a) });
    });
  });
  // labels are 140 x 32: nudge any two that touch apart vertically (and off the center card) until none do
  const NW = 146, NH = 38;
  const hits = (a: { x: number; y: number }, b: { x: number; y: number }) => Math.abs(a.x - b.x) < NW && Math.abs(a.y - b.y) < NH;
  for (let it = 0; it < 60; it++) {
    let moved = false;
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i], b = nodes[j];
        if (!hits(a, b)) continue;
        const d = (NH - Math.abs(a.y - b.y)) / 2 + 1;
        if (a.y <= b.y) { a.y -= d; b.y += d; } else { a.y += d; b.y -= d; }
        moved = true;
      }
      const n = nodes[i];
      if (Math.abs(n.x - cx) < 110 + NW / 2 && Math.abs(n.y - cy) < 30 + NH / 2) { n.y += n.y < cy ? -6 : 6; moved = true; }
      n.y = Math.max(20, Math.min(H - 50, n.y));
    }
    if (!moved) break;
  }
  return (
    <View testID="joke-radial" style={{ width: W, height: H, alignSelf: 'center' }}>
      <Svg width={W} height={H} style={{ position: 'absolute', left: 0, top: 0 }}>
        {nodes.map((n, i) => <Line key={i} x1={cx} y1={cy} x2={n.x} y2={n.y} stroke={GROUP_COLOR[n.g]} strokeOpacity={0.45} strokeWidth={1.5} />)}
        <Circle cx={cx} cy={cy} r={8} fill={C.amber} />
      </Svg>
      <View style={{ position: 'absolute', left: cx - 110, top: cy - 44, width: 220, alignItems: 'center' }} pointerEvents="none">
        <View style={[s.card, { paddingVertical: 8, paddingHorizontal: 12, borderColor: C.amber }]}><T v="semi" numberOfLines={2} style={{ textAlign: 'center' }}>{map.joke.title}</T></View>
      </View>
      {nodes.map((n, i) => (
        <Pressable key={i} testID={`radial-${n.g}-${i}`} accessibilityRole="button" accessibilityLabel={n.b.title} onPress={() => onOpen(n.b, n.g)}
          style={{ position: 'absolute', left: n.x - 70, top: n.y - 16, width: 140, minHeight: 32, borderRadius: 18, borderWidth: 1, borderColor: GROUP_COLOR[n.g], backgroundColor: C.surface, paddingHorizontal: 10, justifyContent: 'center' }}>
          <Text numberOfLines={1} style={{ fontFamily: F.medium, fontSize: 13, color: C.text, textAlign: 'center' }}>{n.b.title}</Text>
        </Pressable>
      ))}
      <View style={{ position: 'absolute', left: 0, bottom: 0, flexDirection: 'row', flexWrap: 'wrap', gap: 12 }}>
        {groups.map((g) => (
          <View key={g.key} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: GROUP_COLOR[g.key] }} />
            <T v="small">{g.title}</T>
          </View>
        ))}
      </View>
    </View>
  );
}

export default function JokeMapScreen() {
  const p = useLocalSearchParams<{ id: string; tab?: string; unit?: string }>();
  const pid = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const wide = useWide();
  const { width } = useWindowDimensions();
  const [trail, setTrail] = useState<{ id: string; title?: string }[]>([{ id: pid }]);
  const [view, setView] = useState<'rows' | 'map'>('rows');
  const [ideas, setIdeas] = useState<{ open: boolean; state?: 'asking' | 'done' | 'error'; r?: any }>({ open: false });
  const [full, setFull] = useState(false);
  const id = trail[trail.length - 1].id;

  // the old workspace links (?tab=…, ?unit=…) still land on the detail page
  useEffect(() => {
    if (p.tab || p.unit) router.replace(`/bit/${encodeURIComponent(pid)}${p.tab ? `?tab=${p.tab}` : ''}${p.unit ? `${p.tab ? '&' : '?'}unit=${encodeURIComponent(String(p.unit))}` : ''}` as any);
  }, [p.tab, p.unit, pid]);
  // the URL follows the walk (and a browser back pops it): derived from the param during render
  const [seenPid, setSeenPid] = useState(pid);
  if (pid !== seenPid) {
    setSeenPid(pid);
    if (pid !== id) setTrail((t) => (t.length > 1 && t[t.length - 2].id === pid ? t.slice(0, -1) : [...t, { id: pid }]));
  }

  const m = useApi<JokeMap>(id ? `/api/jokes/${encodeURIComponent(id)}/map?cap=8` : null);
  const map = m.data && m.data.joke?.joke_id === id ? m.data : null;
  const recenter = (nid: string) => {
    if (!nid || nid === id) return;
    setTrail((t) => [...t.slice(0, -1), { id, title: map?.joke.title }, { id: nid }]);
    setSeenPid(nid);
    setIdeas({ open: false });
    setFull(false);
    router.setParams({ id: nid } as any);
  };
  const back = () => {
    if (trail.length < 2) return;
    const prev = trail[trail.length - 2].id;
    setTrail((t) => t.slice(0, -1));
    setSeenPid(prev);
    setIdeas({ open: false });
    setFull(false);
    router.setParams({ id: prev } as any);
  };
  const open = (b: BitCard, g: MapGroupKey) => {
    if (g === 'versions') return router.push(`/bit/${encodeURIComponent(id)}?tab=versions${b.unit_id ? `&unit=${encodeURIComponent(b.unit_id)}` : ''}` as any);
    if (b.joke_id) recenter(b.joke_id);
  };
  const startSet = async () => {
    if (!map) return;
    try {
      const v = await creative.createBoard({ title: `${map.joke.title} set`, joke_ids: [id], target_s: 720 });
      boards.setCurrent({ id: v.board.id, title: v.board.title });
      router.push(`/board/${encodeURIComponent(v.board.id)}` as any);
    } catch (e: any) {
      ui.snack(`Couldn't start the set: ${e.message}`);
    }
  };
  const toggleIdeas = async () => {
    const opening = !ideas.open;
    setIdeas((x) => ({ ...x, open: opening }));
    if (!opening || ideas.state === 'done') return;
    setIdeas({ open: true, state: 'asking' });
    try {
      const r = await draft<any>(() => post<any>(`/api/jokes/${encodeURIComponent(id)}/punchup`, {}), () => setIdeas({ open: true, state: 'done', r: { status: 'queued' } }));
      setIdeas({ open: true, state: 'done', r });
    } catch (e: any) {
      setIdeas({ open: true, state: 'error', r: { why: e.message } });
    }
  };

  const J = map?.joke;
  const prevTitle = trail.length > 1 ? trail[trail.length - 2].title || 'the last joke' : null;

  const center = J ? (
    <Card testID="map-center" style={{ gap: 10, borderColor: C.amberMid }}>
      {prevTitle && <TextBtn testID="map-back" label={`‹ Back to ${prevTitle}`} color={C.mut} onPress={back} />}
      <T v="h1" testID="map-title">{J.title}</T>
      <T v="small">{[monthYear(J.date), J.stage_line].filter(Boolean).join(' · ') || 'Written, not on tape yet'}</T>
      {!!J.quote && (
        <View style={s.punch}>
          <Text testID="map-quote" maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }}>
            {full && J.text ? J.text.replace(/\s+/g, ' ') : quoteText(J)}
          </Text>
        </View>
      )}
      {!!J.text && J.text.length > (J.quote || '').length + 20 && <TextBtn testID="map-full" label={full ? 'Less' : 'Full text'} color={C.mut} onPress={() => setFull((f) => !f)} />}
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
        <Btn testID="start-set" v="primary" small label="Start a set from here" onPress={startSet} />
        <Btn testID="map-add" v="add" small label="＋ Board" onPress={() => boards.pick(id, J.title)} />
        <Btn testID="map-riff" small label="Riff on this" onPress={() => router.navigate({ pathname: '/', params: { idea: J.title } } as any)} />
        <Btn testID="map-details" small v="ghost" label="The bit in detail ›" onPress={() => router.push(`/bit/${encodeURIComponent(id)}` as any)} />
      </View>
      <MergeHint jokeId={id} onMerged={(mid) => (mid !== id ? recenter(mid) : m.reload())} />
    </Card>
  ) : null;

  const rows = map ? (
    <View testID="map-rows" style={{ gap: 16 }}>
      {map.groups.filter((g) => g.items.length).map((g) => (
        <View key={g.key} testID={`group-${g.key}`} style={{ gap: 8 }}>
          <View style={s.row}>
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: GROUP_COLOR[g.key] }} />
            <T v="eb" style={{ flex: 1 }}>{g.title}</T>
          </View>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} style={{ marginHorizontal: wide ? 0 : -16, flexGrow: 0 }}
            contentContainerStyle={{ gap: 10, paddingHorizontal: wide ? 0 : 16 }}>
            {g.items.map((b, i) => <MapCard key={`${b.joke_id}-${b.unit_id || ''}-${i}`} b={b} g={g.key} i={i} onOpen={() => open(b, g.key)} onAdd={() => boards.pick(b.joke_id, b.title)} />)}
          </ScrollView>
        </View>
      ))}
      {!map.groups.some((g) => g.items.length) && <Empty text="Nothing connects to this one yet. Riff on it, or start a set from here." />}
    </View>
  ) : null;

  const ideasBlock = J ? (
    <View testID="map-ideas" style={[s.draft, { padding: 0 }]}>
      <Pressable testID="map-ideas-toggle" accessibilityRole="button" accessibilityState={{ expanded: ideas.open }} onPress={toggleIdeas}
        style={{ minHeight: 48, paddingHorizontal: 12, justifyContent: 'center' }}>
        <T v="small" style={{ color: C.amber }}>{ideas.open ? '▾' : '▸'} Ideas from the AI · not your words yet</T>
      </Pressable>
      {ideas.open && (
        <View style={{ gap: 8, paddingHorizontal: 12, paddingBottom: 12 }}>
          {ideas.state === 'asking' && <T v="mut">Asking…</T>}
          {ideas.state === 'error' && <T v="mut">The AI didn’t answer.</T>}
          {ideas.state === 'done' && ideas.r?.status === 'queued' && <T v="mut" testID="map-ideas-queued">Your home worker drafts these. They show up here when it’s done.</T>}
          {ideas.state === 'done' && ideas.r?.status === 'no_tape' && <T v="mut">Ideas come from how it plays on stage. This one isn’t on tape yet.</T>}
          {ideas.state === 'done' && ideas.r?.status === 'locked' && <T v="mut">You locked this bit. No drafts.</T>}
          {ideas.state === 'done' && (ideas.r?.drafts || []).map((d: any, i: number) => (
            <View key={i} testID={`map-idea-${i}`} style={{ gap: 4 }}>
              <View style={{ alignSelf: 'flex-start', paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.addBg, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft</Text></View>
              <Text style={{ fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.amber }}>{d.alt}</Text>
            </View>
          ))}
        </View>
      )}
    </View>
  ) : null;

  const mapW = Math.min(1000, width - (wide ? 232 + 64 : 32));
  return (
    <Screen title="Joke" back testID="joke-map"
      right={wide ? <View style={{ paddingRight: 8 }}><Seg testID="map-view" fit items={[['rows', 'Rows'], ['map', 'Map']]} value={view} onChange={setView} /></View> : undefined}>
      {m.error && !m.data ? <ErrorCard error={m.error} onRetry={m.reload} /> : null}
      {!map && !m.error && <Empty text="Opening the joke…" />}
      {center}
      {wide && view === 'map' && map ? <Radial map={map} width={mapW} onOpen={open} /> : rows}
      {ideasBlock}
    </Screen>
  );
}

