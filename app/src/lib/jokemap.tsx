// The bird's-eye view of a joke's map (lib/simple JokeMap): the joke in the middle, each group in its own slice of
// the circle. Drawn on the bit page (/joke/<id>) and the full map (/map/<id>).
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import Svg, { Circle, Line } from 'react-native-svg';
import { BitCard, JokeMap, MapGroupKey } from './simple';
import { C, F } from './theme';
import { T, s } from './ui';

export const GROUP_COLOR: Record<MapGroupKey, string> = { next: C.amber, tags: C.green, same_world: C.blue, callbacks: C.redSoft, versions: C.mut };

/** The bird's-eye view: the joke in the middle, each group in its own slice of the circle. */
export function Radial({ map, onOpen, width }: { map: JokeMap; onOpen: (b: BitCard, g: MapGroupKey) => void; width: number }) {
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

