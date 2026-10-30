// Laugh-metric components (UX_SPEC 5.2). Every one carries its meaning in shape, count or text as well as colour,
// and every number goes through the rounding rules in laughs.ts.
import React, { memo, useState } from 'react';
import { LayoutChangeEvent, Pressable, Text, View } from 'react-native';
import Svg, { Circle, Defs, Line, Pattern, Rect, Path } from 'react-native-svg';
import { C, F } from './theme';
import { BAR_WORDS, RECEPTION_WORDS, Rating, fmt1, heatStep, heatWord, receptionBars } from '@/rules/laughs';
import { T } from './ui';

// ------------------------------------------------------------------ headline
/** "5.1 laugh-s/min · about your usual". Tapping it opens the Why sheet (the only place raw laughs/min appears). */
export function LaughHeadline({ value, roomHeat, onPress, testID, size = 'big', sub }: { value: number | null | undefined; roomHeat?: number | null; onPress?: () => void; testID?: string; size?: 'big' | 'mid'; sub?: string | null }) {
  const word = heatWord(roomHeat);
  const big = size === 'big';
  return (
    <Pressable testID={testID} disabled={!onPress} onPress={onPress} accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={`${fmt1(value)} seconds of laughter per minute${word ? `, ${word}` : ''}${onPress ? '. Why?' : ''}`}
      style={{ flexDirection: 'row', alignItems: 'baseline', flexWrap: 'wrap', columnGap: 10, rowGap: 2, minHeight: 44 }}>
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g7, fontSize: big ? 40 : 28, lineHeight: big ? 44 : 32, letterSpacing: -1, color: C.text, fontVariant: ['tabular-nums'] }}>{fmt1(value)}</Text>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g6, fontSize: big ? 16 : 14, color: C.text2 }}>laugh-s/min</Text>
      {!!word && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: big ? 15 : 14, color: word === 'hot room' ? C.amber : C.mut }}>{word}</Text>}
      {!!sub && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.dim }}>{sub}</Text>}
    </Pressable>
  );
}

// ------------------------------------------------------------------ reception meter (0-4 bars)
/** 0–4 bars: the count of filled bars and their rising heights carry the meaning, never the colour alone. */
export const ReceptionMeter = memo(function ReceptionMeter({ bars, size = 16, word, testID }: { bars: number; size?: number; word?: string | null; testID?: string }) {
  const b = Math.max(0, Math.min(4, Math.round(bars)));
  const w = Math.max(3, size / 3.2);
  return (
    <View testID={testID} accessibilityLabel={`${b} of 4 bars${word ? `, ${word}` : ''}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 2, height: size }}>
        {[1, 2, 3, 4].map((k) => (
          <View key={k} style={{ width: w, height: (size * k) / 4, borderRadius: 1, borderWidth: 1, borderColor: k <= b ? C.amber : C.borderStrong, backgroundColor: k <= b ? C.amber : 'transparent' }} />
        ))}
      </View>
      {word != null && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.text2 }}>{word}</Text>}
    </View>
  );
});
/** The legend: every level as bars + words. */
export function ReceptionLegend({ kind = 'room', testID }: { kind?: 'room' | 'laugh'; testID?: string }) {
  const words = kind === 'room' ? RECEPTION_WORDS : BAR_WORDS;
  return (
    <View testID={testID} style={{ flexDirection: 'row', flexWrap: 'wrap', columnGap: 14, rowGap: 6 }}>
      {words.map((w, i) => <ReceptionMeter key={w} bars={i} size={12} word={`${i} · ${w}`} />)}
    </View>
  );
}
export const roomBars = receptionBars;

// ------------------------------------------------------------------ × room bar
/** Bar length = vs_room capped at 3×, with a tick at 1× (the room's own rate). Hatched when the room is too quiet. */
export const VsRoomBar = memo(function VsRoomBar({ vs, comparable, testID, width }: { vs: number | null | undefined; comparable: boolean; testID?: string; width?: number }) {
  const [w, setW] = useState(width || 0);
  const on = (e: LayoutChangeEvent) => !width && setW(e.nativeEvent.layout.width);
  const h = 10;
  if (!comparable) {
    return (
      <View testID={testID} onLayout={on} accessibilityLabel="room too quiet to compare" style={{ gap: 2, flex: width ? undefined : 1, width }}>
        {w > 0 && (
          <Svg width={w} height={h}>
            <Defs>
              <Pattern id="hatch" width={6} height={6} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                <Line x1={0} y1={0} x2={0} y2={6} stroke={C.borderStrong} strokeWidth={2} />
              </Pattern>
            </Defs>
            <Rect x={0} y={0} width={w} height={h} rx={3} fill="url(#hatch)" />
          </Svg>
        )}
        <Text style={{ fontFamily: F.body, fontSize: 12, color: C.dim }}>room too quiet to compare</Text>
      </View>
    );
  }
  const v = Math.max(0, Math.min(3, vs || 0));
  return (
    <View testID={testID} onLayout={on} accessibilityLabel={`${fmt1(vs)} times the room`} style={{ flex: width ? undefined : 1, width, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
      <View style={{ flex: 1, height: h }}>
        {w > 0 && (
          <Svg width={Math.max(0, w - 54)} height={h}>
            <Rect x={0} y={0} width={Math.max(0, w - 54)} height={h} rx={3} fill={C.surface2} />
            <Rect x={0} y={0} width={(Math.max(0, w - 54) * v) / 3} height={h} rx={3} fill={C.amber} />
            <Line x1={(Math.max(0, w - 54)) / 3} y1={-1} x2={(Math.max(0, w - 54)) / 3} y2={h + 1} stroke={C.text} strokeWidth={1.5} />
          </Svg>
        )}
      </View>
      <Text maxFontSizeMultiplier={1.3} style={{ width: 46, textAlign: 'right', fontFamily: F.g6, fontSize: 13, color: C.text2, fontVariant: ['tabular-nums'] }}>{fmt1(vs)}×</Text>
    </View>
  );
});

// ------------------------------------------------------------------ rating tag
const RCOL: Record<Rating, string> = { killed: C.amber, worked: C.text, meh: C.mut, died: C.redSoft };
/** A rating word, derived from the numbers (laughs.ts displayRating). A word, not a traffic light. */
export function RatingTag({ word, yours, testID }: { word: Rating | null; yours?: boolean; testID?: string }) {
  if (!word) return null;
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 7, minHeight: 22, borderRadius: 4, borderWidth: 1, borderColor: C.border }}>
      {yours && <View style={{ width: 5, height: 5, borderRadius: 3, backgroundColor: C.amber }} />}
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g6, fontSize: 12, color: RCOL[word] }}>{word}</Text>
    </View>
  );
}

// ------------------------------------------------------------------ timeline density strip (Set)
export type StripSeg = { idx: number; start_ms: number; end_ms: number; lspm: number | null; nonBit: boolean; word?: Rating | null };
/**
 * Segments laid out by time; fill opacity = the segment's laugh-s/min relative to the set's max; laugh events as
 * ticks above (height = bars); banter/crowd segments in a neutral hatch; the longest quiet marked with a bracket.
 * Rating colours only as a 3 px underline. Tap a segment to jump to its row.
 */
export function DensityStrip({ durationMs, segs, laughs, quiet, onPress, testID }: {
  durationMs: number; segs: StripSeg[]; laughs: { start_ms: number; end_ms: number; bars: number }[]; quiet?: { at_ms: number; dur_s: number } | null;
  onPress?: (idx: number) => void; testID?: string;
}) {
  const [w, setW] = useState(0);
  const H = 56;
  const tickH = 16;
  const max = Math.max(0.1, ...segs.filter((x) => !x.nonBit).map((x) => x.lspm || 0));
  const X = (ms: number) => (w * Math.max(0, Math.min(durationMs, ms))) / Math.max(1, durationMs);
  return (
    <View testID={testID} onLayout={(e) => setW(e.nativeEvent.layout.width)} style={{ height: H + 20 }} accessibilityLabel="Laugh density across the set">
      {w > 0 && (
        <Svg width={w} height={H}>
          <Defs>
            <Pattern id="grey" width={5} height={5} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <Line x1={0} y1={0} x2={0} y2={5} stroke={C.border} strokeWidth={2} />
            </Pattern>
          </Defs>
          {laughs.map((l, i) => {
            const th = (tickH * Math.max(1, l.bars)) / 4;
            return <Rect key={`l${i}`} x={X(l.start_ms)} y={tickH - th} width={Math.max(2, X(l.end_ms) - X(l.start_ms))} height={th} fill={C.amber} />;
          })}
          {segs.map((sg) => {
            const x = X(sg.start_ms);
            const sw = Math.max(1.5, X(sg.end_ms) - x - 1);
            if (sg.nonBit) return <Rect key={sg.idx} x={x} y={tickH + 4} width={sw} height={24} fill="url(#grey)" />;
            const a = Math.max(0.08, Math.min(1, (sg.lspm || 0) / max));
            return (
              <React.Fragment key={sg.idx}>
                <Rect x={x} y={tickH + 4} width={sw} height={24} fill={C.amber} fillOpacity={a} stroke={C.amber} strokeOpacity={0.35} strokeWidth={0.5} />
                {sg.word && <Rect x={x} y={tickH + 31} width={sw} height={3} fill={RCOL[sg.word]} />}
              </React.Fragment>
            );
          })}
          {quiet && quiet.dur_s >= 30 && (
            <Path d={`M${X(quiet.at_ms)} ${H - 6} v4 h${Math.max(2, X(quiet.at_ms + quiet.dur_s * 1000) - X(quiet.at_ms))} v-4`} stroke={C.redSoft} strokeWidth={1.5} fill="none" />
          )}
        </Svg>
      )}
      {w > 0 && onPress && segs.map((sg) => (
        <Pressable key={`p${sg.idx}`} testID={`seg-${sg.idx}`} accessibilityRole="button" accessibilityLabel={`Jump to row ${sg.idx}`} onPress={() => onPress(sg.idx)}
          style={{ position: 'absolute', left: X(sg.start_ms), width: Math.max(4, X(sg.end_ms) - X(sg.start_ms)), top: 0, height: H }} />
      ))}
      <View style={{ flexDirection: 'row', justifyContent: 'space-between', marginTop: 4 }}>
        <Text style={{ fontFamily: F.g5, fontSize: 11, color: C.dim }}>0:00</Text>
        {quiet && quiet.dur_s >= 30 && <Text style={{ fontFamily: F.g5, fontSize: 11, color: C.redSoft }}>└ longest quiet</Text>}
        <Text style={{ fontFamily: F.g5, fontSize: 11, color: C.dim }}>{`${Math.floor(durationMs / 60000)}:${String(Math.floor((durationMs % 60000) / 1000)).padStart(2, '0')}`}</Text>
      </View>
    </View>
  );
}

// ------------------------------------------------------------------ sparkline (Joke · On stage)
export type SparkNight = { key: string; label: string; value: number; excluded?: boolean; best?: boolean };
/** One dot per night, equal spacing, venue + year under each dot, the best night ringed, unreliable nights hollow grey. */
export function Sparkline({ nights, onPress, testID, height = 64 }: { nights: SparkNight[]; onPress?: (k: string) => void; testID?: string; height?: number }) {
  const [w, setW] = useState(0);
  const n = nights.length;
  const max = Math.max(0.1, ...nights.filter((x) => !x.excluded).map((x) => x.value));
  const pad = 14;
  const X = (i: number) => (n <= 1 ? w / 2 : pad + ((w - 2 * pad) * i) / (n - 1));
  const Y = (v: number) => pad + (height - 2 * pad) * (1 - Math.min(1, v / max));
  const pts = nights.map((x, i) => ({ ...x, x: X(i), y: Y(x.value) }));
  const line = pts.filter((p) => !p.excluded).map((p, i) => `${i ? 'L' : 'M'}${p.x} ${p.y}`).join(' ');
  return (
    <View testID={testID} onLayout={(e) => setW(e.nativeEvent.layout.width)} accessibilityLabel={`Laugh-seconds per minute over ${n} nights`}>
      <View style={{ height }}>
        {w > 0 && (
          <Svg width={w} height={height}>
            {!!line && <Path d={line} stroke={C.amber} strokeOpacity={0.5} strokeWidth={1.5} fill="none" />}
            {pts.map((p) => (
              <React.Fragment key={p.key}>
                {p.best && <Circle cx={p.x} cy={p.y} r={9} stroke={C.amber} strokeWidth={1.5} fill="none" />}
                <Circle cx={p.x} cy={p.y} r={p.best ? 5 : 4} fill={p.excluded ? 'none' : C.amber} stroke={p.excluded ? C.dim : C.amber} strokeWidth={1.5} />
              </React.Fragment>
            ))}
          </Svg>
        )}
        {w > 0 && onPress && pts.map((p) => (
          <Pressable key={`p${p.key}`} testID={`spark-${p.key}`} accessibilityRole="button" accessibilityLabel={`${p.label}: ${fmt1(p.value)} laugh-s/min${p.excluded ? ', audio unreliable' : ''}`}
            onPress={() => onPress(p.key)} style={{ position: 'absolute', left: p.x - Math.max(22, (w / Math.max(1, n)) / 2), width: Math.max(44, w / Math.max(1, n)), top: 0, height }} />
        ))}
      </View>
      {w > 0 && (
        <View style={{ height: 30 }}>
          {pts.map((p) => (
            <Text key={`t${p.key}`} numberOfLines={2} maxFontSizeMultiplier={1.2}
              style={{ position: 'absolute', left: p.x - 34, width: 68, textAlign: 'center', fontFamily: F.g5, fontSize: 10, lineHeight: 13, color: p.excluded ? C.dim : p.best ? C.amber : C.mut }}>{p.label}</Text>
          ))}
        </View>
      )}
    </View>
  );
}

// ------------------------------------------------------------------ small meters
/** Desk strength dots: count carries the meaning; hollow outline = never performed. */
export const StrengthDots = memo(function StrengthDots({ n, testID }: { n: 0 | 1 | 2 | 3 | 4; testID?: string }) {
  const never = n === 0;
  return (
    <View testID={testID} accessibilityLabel={never ? 'never performed' : `strength ${n} of 4`} style={{ flexDirection: 'row', gap: 3 }}>
      {[1, 2, 3, 4].map((k) => (
        <View key={k} style={{ width: 7, height: 7, borderRadius: 4, borderWidth: 1, borderColor: never ? C.dim : k <= n ? C.amber : C.borderStrong, backgroundColor: !never && k <= n ? C.amber : 'transparent' }} />
      ))}
    </View>
  );
});

/** Working-set nights strip: one cell per night; fill HEIGHT = band (< 0.8× third, 0.8–1.3× two thirds, ≥ 1.3× full); empty = not performed. */
export const MiniStrip = memo(function MiniStrip({ bands, testID, size = 14 }: { bands: (0 | 1 | 2 | null)[]; testID?: string; size?: number }) {
  const word = (b: 0 | 1 | 2 | null) => (b == null ? 'not performed' : b === 0 ? 'under the room' : b === 1 ? 'with the room' : 'above the room');
  return (
    <View testID={testID} accessibilityLabel={bands.map(word).join(', ')} style={{ flexDirection: 'row', gap: 2 }}>
      {bands.map((b, i) => (
        <View key={i} style={{ width: size, height: size, borderWidth: 1, borderColor: b == null ? C.border : C.addBorder, borderRadius: 2, justifyContent: 'flex-end', overflow: 'hidden' }}>
          {b != null && <View style={{ height: `${((b + 1) / 3) * 100}%`, backgroundColor: C.amber }} />}
        </View>
      ))}
    </View>
  );
});

/** Storyboard heat meter: 4 rising bars from avg × room (a sketching surface, so no numbers). */
export const HeatMeter = memo(function HeatMeter({ heat, testID }: { heat: number; testID?: string }) {
  return (
    <View testID={testID} accessibilityLabel={heat ? `heat ${heat} of 4` : 'not on tape'} style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 2, height: 16 }}>
      {[1, 2, 3, 4].map((k) => (
        <View key={k} style={{ width: 4, height: 4 * k, borderRadius: 1, backgroundColor: k <= heat ? C.amber : C.border }} />
      ))}
    </View>
  );
});

/** Perfected: locked seconds amber-solid, open seconds outline, plus the stability word. Never one fused score. */
export function PerfectedMeter({ segments, lockedShare, word, testID }: { segments: { key: string; s: number; locked: boolean }[]; lockedShare: number; word: string; testID?: string }) {
  const total = segments.reduce((a, x) => a + x.s, 0) || 1;
  return (
    <View testID={testID} style={{ gap: 6 }} accessibilityLabel={`Perfected ${Math.round(lockedShare * 100)} percent locked, ${word}`}>
      <View style={{ flexDirection: 'row', gap: 2, height: 10 }}>
        {segments.map((x) => (
          <View key={x.key} style={{ flex: x.s / total, borderRadius: 2, borderWidth: 1, borderColor: C.amber, backgroundColor: x.locked ? C.amber : 'transparent' }} />
        ))}
      </View>
      <T v="small"><Text style={{ fontFamily: F.g6, color: C.text }}>Perfected {Math.round(lockedShare * 100)}%</Text> locked · {word}</T>
    </View>
  );
}

/** Heat as a glyph too (never colour alone): ○ quiet · ◐ usual · ● hot. */
const HEAT_GLYPH = ['○', '◐', '●'];
export function HeatDot({ heat }: { heat: number | null | undefined }) {
  const h = heatStep(heat);
  if (h == null) return null;
  return <Text accessibilityLabel={heatWord(heat) || ''} style={{ color: h === 2 ? C.amber : h === 1 ? C.amberMid : C.mut, fontSize: 13 }}>{HEAT_GLYPH[h]}</Text>;
}

