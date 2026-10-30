// The Punchable mark (concept C, "Laugh meter", the comic's pick 2026-10-26: docs/design/DESIGN.md §7) and the wordmark. Geometry = app/scripts/brand/brand.mjs.
import { Text, View, type StyleProp, type ViewStyle } from 'react-native';
import Svg, { Path, Rect } from 'react-native-svg';
import { C, F } from './theme';

// The app's 4-bar reception meter, peaking: [path, opacity] per bar (amber heat ramp, theme.ts C.heat).
const BARS: [string, number][] = [
  ['M96 338a22 22 0 0 1 22-22h20a22 22 0 0 1 22 22V436H96z', 0.35],
  ['M192 258a22 22 0 0 1 22-22h20a22 22 0 0 1 22 22V436H192z', 0.6],
  ['M288 178a22 22 0 0 1 22-22h20a22 22 0 0 1 22 22V436H288z', 0.85],
  ['M384 98a22 22 0 0 1 22-22h20a22 22 0 0 1 22 22V436H384z', 1],
];

/** The mark on its graphite tile (the app icon), `size` px square. */
export function MeterMark({ size = 56, tile = C.surface }: { size?: number; tile?: string }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 512 512" aria-hidden>
      <Rect width={512} height={512} rx={112} fill={tile} />
      {BARS.map(([d, o]) => <Path key={d} d={d} fill={C.amber} opacity={o} />)}
    </Svg>
  );
}

/** Mark + "punchable", read by screen readers as one word. */
export function Lockup({ size = 56, style }: { size?: number; style?: StyleProp<ViewStyle> }) {
  return (
    <View accessible accessibilityRole="header" accessibilityLabel="Punchable" style={[{ flexDirection: 'row', alignItems: 'center', gap: size * 0.25 }, style]}>
      <MeterMark size={size} />
      <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.g7, fontSize: size * 0.54, letterSpacing: -size * 0.016, color: C.text }}>punchable</Text>
    </View>
  );
}
