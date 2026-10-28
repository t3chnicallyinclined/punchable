// Punchable UI kit: the "Backstage Amber" design system as React Native primitives, plus the app shell
// (SIMPLE.md: Write · Boards · Notebook; the avatar opens You: profile, sign out, settings. Shows, sets, the map,
// import and record sit one tap down in Notebook › Shows).
import React, { createContext, memo, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import {
  AccessibilityInfo, Image, KeyboardAvoidingView, Modal, Platform, Pressable, RefreshControl, ScrollView, StyleProp, Text, TextProps, TextStyle, View, ViewStyle, useWindowDimensions,
} from 'react-native';
import { router, usePathname } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import Svg, { Circle, Path } from 'react-native-svg';
import { C, F, WIDE, onSchemeChange, stageInfo, stageWord } from './theme';
import { AskButton } from './AskButton';
import { upload } from './api';
import { useSession } from './auth';
import { loginHref } from './session';
import { isWeb, micExplained, micSupported, pickFile, requestMic, setMicExplained } from './media';

// ------------------------------------------------------------------ text (scales with the OS text size; capped)
type TV = 'body' | 'mut' | 'eb' | 'h1' | 'h2' | 'num' | 'semi' | 'small' | 'big';
// Body text is 16 on the web and 17 on iOS (the HIG default), scaling with the OS text size (capped at 1.6x).
const BODY = Platform.OS === 'ios' ? 17 : 16;
const buildTv = (): Record<TV, TextStyle> => ({
  body: { fontFamily: F.body, fontSize: BODY, lineHeight: BODY + 7, color: C.text },
  semi: { fontFamily: F.semi, fontSize: BODY, lineHeight: BODY + 7, color: C.text },
  mut: { fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.mut },
  small: { fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut },
  eb: { fontFamily: F.g7, fontSize: 12, letterSpacing: 0.96, textTransform: 'uppercase', color: C.mut },
  h1: { fontFamily: F.g7, fontSize: 30, lineHeight: 35, letterSpacing: -0.6, color: C.text },
  h2: { fontFamily: F.g6, fontSize: 20, lineHeight: 26, color: C.text },
  num: { fontFamily: F.g6, fontVariant: ['tabular-nums'], color: C.text, fontSize: 16 },
  big: { fontFamily: F.g7, fontVariant: ['tabular-nums'], color: C.text, fontSize: 40, lineHeight: 44, letterSpacing: -1 },
});
const tv: Record<TV, TextStyle> = buildTv();
export function T({ v = 'body', style, ...p }: TextProps & { v?: TV }) {
  return <Text maxFontSizeMultiplier={1.6} {...p} style={[tv[v], style]} />;
}

// ------------------------------------------------------------------ boxes
// Static styles read the tokens once; the scheme change (theme.setScheme) rebuilds them in place and the root
// remounts, so a light/dark switch reaches these too.
const buildS = () => ({
  card: { backgroundColor: C.surface, borderColor: C.border, borderWidth: 1, borderRadius: 12, padding: 16 } as ViewStyle,
  inner: { backgroundColor: C.surface2, borderColor: C.border, borderWidth: 1, borderRadius: 10, padding: 12 } as ViewStyle,
  row: { flexDirection: 'row', alignItems: 'center', gap: 12 } as ViewStyle,
  sep: { borderTopWidth: 1, borderTopColor: C.border } as ViewStyle,
  punch: { borderLeftWidth: 3, borderLeftColor: C.amber, paddingLeft: 12 } as ViewStyle,
  rv: { backgroundColor: C.amber, color: C.bg, fontFamily: F.semi, borderRadius: 4, paddingHorizontal: 4 } as TextStyle,
  draft: { borderWidth: 1.5, borderStyle: 'dashed', borderColor: C.draftBorder, borderRadius: 10, padding: 12, backgroundColor: C.draftBg } as ViewStyle,
});
export const s = buildS();
onSchemeChange(() => {
  Object.assign(s, buildS());
  Object.assign(tv, buildTv());
});

/** Reduce Motion (HIG): fades instead of slides, no spring on the map. Read once; the OS setting rarely changes. */
let reduceMotion = false;
try {
  AccessibilityInfo.isReduceMotionEnabled?.().then((b) => { reduceMotion = !!b; }).catch(() => {});
} catch {}
export const prefersReducedMotion = () => reduceMotion;
export function useReduceMotion() {
  const [r, setR] = useState(reduceMotion);
  useEffect(() => {
    let alive = true;
    try {
      AccessibilityInfo.isReduceMotionEnabled?.().then((b) => alive && setR(!!b)).catch(() => {});
      const sub = AccessibilityInfo.addEventListener?.('reduceMotionChanged', (b: boolean) => { reduceMotion = b; if (alive) setR(b); });
      return () => { alive = false; sub?.remove?.(); };
    } catch {
      return () => { alive = false; };
    }
  }, []);
  return r;
}

/** A loading placeholder (HIG: skeletons, not spinners mid-screen). */
export function Skeleton({ lines = 3, height = 14, style, testID }: { lines?: number; height?: number; style?: StyleProp<ViewStyle>; testID?: string }) {
  return (
    <View testID={testID} accessibilityLabel="Loading" accessibilityRole={'progressbar' as any} style={[{ gap: 10 }, style]}>
      {Array.from({ length: lines }).map((_, i) => (
        <View key={i} style={{ height, borderRadius: height / 2, backgroundColor: C.surface2, width: i === lines - 1 && lines > 1 ? '62%' : '100%' }} />
      ))}
    </View>
  );
}

export function Card({ style, children, testID }: { style?: StyleProp<ViewStyle>; children?: React.ReactNode; testID?: string }) {
  return <View testID={testID} style={[s.card, style]}>{children}</View>;
}

export function Badge({ label, fg = C.text, bg = 'transparent', bordered, style }: { label: string; fg?: string; bg?: string; bordered?: boolean; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={[{ minHeight: 24, paddingHorizontal: 8, borderRadius: 4, backgroundColor: bg, justifyContent: 'center', alignSelf: 'flex-start' },
      bordered && { borderWidth: 1, borderColor: C.border }, style]}>
      <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.g7, fontSize: 11, letterSpacing: 0.66, textTransform: 'uppercase', color: fg }}>{label}</Text>
    </View>
  );
}
export const StageBadge = ({ stage, ai }: { stage?: string | null; ai?: boolean }) => {
  const m = stageInfo(stage);
  return <Badge label={ai ? `${m.label} (AI)` : m.label} fg={m.fg} bg={m.bg} />;
};
/** The stage as a coloured word; a 6 px blue dot before it when the AI set it and he hasn't touched it. */
export function StageWord({ stage, ai, testID }: { stage?: string | null; ai?: boolean; testID?: string }) {
  const m = stageInfo(stage);
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 5 }} accessibilityLabel={`${stageWord(stage)}${ai ? ', set by the AI' : ''}`}>
      {ai && <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.blue }} />}
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g6, fontSize: 13, color: m.fg }}>{stageWord(stage)}</Text>
    </View>
  );
}

/** The prototype's `.ai` chip, on the claim (not on every word). `word` = the confidence in words (8.3). */
export function AIBadge({ conf, word, testID, style }: { conf?: number | null; word?: string | null; testID?: string; style?: StyleProp<ViewStyle> }) {
  const w = word ?? null;
  return (
    <View testID={testID} accessibilityLabel={`AI${w ? `, ${w}` : ''}`}
      style={[{ flexDirection: 'row', alignItems: 'center', gap: 4, minHeight: 18, paddingHorizontal: 5, borderRadius: 4, borderWidth: 1, borderColor: C.aiBorder, backgroundColor: C.aiBg, alignSelf: 'center' }, style]}>
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g7, fontSize: 11, letterSpacing: 0.6, color: C.blue }}>AI</Text>
      {!!w && <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g6, fontSize: 11, color: C.blue }}>{w}</Text>}
    </View>
  );
}
/** He edited it: the badge becomes a small "yours" mark. */
export function Yours({ testID }: { testID?: string }) {
  return (
    <View testID={testID} accessibilityLabel="yours" style={{ flexDirection: 'row', alignItems: 'center', gap: 4, alignSelf: 'center' }}>
      <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.amber }} />
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g6, fontSize: 11, color: C.amber }}>yours</Text>
    </View>
  );
}
/** "✓ on stage": the pair was performed adjacently or he confirmed it. A tag, not a button. */
export function Proven({ label = 'on stage' }: { label?: string }) {
  return (
    <View accessibilityLabel={`proven ${label}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 3, paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.greenSoft, alignSelf: 'center' }}>
      <Text style={{ fontFamily: F.g7, fontSize: 11, color: C.green }}>✓ {label}</Text>
    </View>
  );
}

/** A small secondary text action. Kept for the optional Step through screen. */
export function SmallAct({ label, onPress, color = C.mut, testID, accessibilityLabel }: { label: string; onPress: () => void; color?: string; testID?: string; accessibilityLabel?: string }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={accessibilityLabel || label} onPress={onPress}
      style={(st: any) => ({ minHeight: 44, minWidth: 44, paddingHorizontal: 10, borderRadius: 10, alignItems: 'center', justifyContent: 'center', backgroundColor: st.hovered ? C.surface2 : 'transparent' })}>
      <Text style={{ fontFamily: F.semi, fontSize: 14, color }}>{label}</Text>
    </Pressable>
  );
}

type BtnV = 'plain' | 'primary' | 'green' | 'ghost' | 'danger' | 'add';
export function Btn({ label, onPress, v = 'plain', style, textStyle, disabled, testID, icon, accessibilityLabel, small, onLongPress }: {
  label: string; onPress?: () => void; v?: BtnV; style?: StyleProp<ViewStyle>; textStyle?: StyleProp<TextStyle>; disabled?: boolean;
  testID?: string; icon?: React.ReactNode; accessibilityLabel?: string; small?: boolean; onLongPress?: () => void;
}) {
  const bg = v === 'primary' ? C.amber : v === 'green' ? C.green : v === 'add' ? C.addBg : 'transparent';
  const bd = v === 'primary' ? C.amber : v === 'green' ? C.green : v === 'ghost' ? 'transparent' : v === 'danger' ? C.dangerBorder : v === 'add' ? C.addBorder : C.border;
  const fg = v === 'primary' ? C.bg : v === 'green' ? C.greenDeep : v === 'danger' ? C.redSoft : v === 'add' ? C.amber : C.text;
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={accessibilityLabel || label} accessibilityState={{ disabled: !!disabled }}
      disabled={disabled} onPress={onPress} onLongPress={onLongPress}
      style={(st: any) => [{
        minHeight: small ? 44 : 48, paddingHorizontal: small ? 12 : 16, borderRadius: 12, borderWidth: 1, borderColor: bd, backgroundColor: bg,
        flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8, opacity: disabled ? 0.45 : st.pressed ? 0.8 : 1,
      }, st.hovered && (v === 'plain' || v === 'ghost') && { backgroundColor: C.surface2 }, style]}>
      {icon}
      <Text maxFontSizeMultiplier={1.4} style={[{ fontFamily: F.g7, fontSize: small ? 14 : 15, letterSpacing: 0.3, color: fg, textAlign: 'center' }, textStyle]}>{label}</Text>
    </Pressable>
  );
}
/** The trailing amber ＋ (add to the current board). 44 px target. */
export const AddBtn = memo(function AddBtn({ onPress, onLongPress, testID, label = 'Add to board' }: { onPress: () => void; onLongPress?: () => void; testID?: string; label?: string }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={label} onPress={onPress} onLongPress={onLongPress} hitSlop={4}
      style={({ pressed }) => ({ width: 44, height: 44, borderRadius: 10, borderWidth: 1, borderColor: C.addBorder, backgroundColor: C.addBg, alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.7 : 1 })}>
      <Text style={{ color: C.amber, fontSize: 20, lineHeight: 22, fontFamily: F.g6 }}>＋</Text>
    </Pressable>
  );
});

export function Chip({ label, on, onPress, testID, count, onRemove }: { label: string; on?: boolean; onPress?: () => void; testID?: string; count?: number; onRemove?: () => void }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityState={{ selected: !!on }} onPress={onRemove || onPress}
      accessibilityLabel={onRemove ? `${label}, remove filter` : label}
      style={{ minHeight: 44, paddingHorizontal: 14, borderRadius: 8, borderWidth: 1, borderColor: on ? C.amber : C.border, backgroundColor: on ? C.amber : 'transparent', flexDirection: 'row', alignItems: 'center', gap: 6 }}>
      <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 14, color: on ? C.bg : C.text2 }}>{label}</Text>
      {count != null && <Text style={{ fontFamily: F.g6, fontSize: 13, color: on ? C.bg : C.mut }}>{count}</Text>}
      {onRemove && <Text style={{ fontFamily: F.g7, fontSize: 14, color: on ? C.bg : C.mut }}>×</Text>}
    </Pressable>
  );
}

export function Chips({ children, testID }: { children: React.ReactNode; testID?: string }) {
  const wide = useWide();
  return (
    <ScrollView testID={testID} horizontal showsHorizontalScrollIndicator={false} style={{ marginHorizontal: wide ? 0 : -16, flexGrow: 0 }}
      contentContainerStyle={{ gap: 8, paddingHorizontal: wide ? 0 : 16 }}>
      {children}
    </ScrollView>
  );
}

export function Seg<K extends string>({ items, value, onChange, testID, fit }: { items: [K, string][]; value: K; onChange: (k: K) => void; testID?: string; fit?: boolean }) {
  return (
    <View testID={testID} accessibilityRole="tablist" style={{ flexDirection: 'row', gap: 2, backgroundColor: C.surface, borderColor: C.border, borderWidth: 1, borderRadius: 10, padding: 3 }}>
      {items.map(([k, l]) => {
        const on = k === value;
        return (
          <Pressable key={k} testID={testID ? `${testID}-${k}` : undefined} accessibilityRole="tab" accessibilityState={{ selected: on }} onPress={() => onChange(k)}
            style={{ flexGrow: 1, flexBasis: fit ? 'auto' : 0, minWidth: 0, minHeight: 44, borderRadius: 8, alignItems: 'center', justifyContent: 'center', paddingHorizontal: fit ? 6 : 4, backgroundColor: on ? C.text : 'transparent' }}>
            <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.g6, fontSize: 14, color: on ? C.bg : C.mut }}>{l}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

export function ListRow({ title, sub, onPress, right, testID, titleColor, chevron = true }: {
  title: React.ReactNode; sub?: React.ReactNode; onPress?: () => void; right?: React.ReactNode; testID?: string; titleColor?: string; chevron?: boolean;
}) {
  return (
    <Pressable testID={testID} accessibilityRole={onPress ? 'button' : undefined} onPress={onPress}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 56, paddingVertical: 10 }}>
      <View style={{ flex: 1, minWidth: 0 }}>
        {typeof title === 'string' ? <T v="semi" style={titleColor ? { color: titleColor } : undefined}>{title}</T> : title}
        {sub != null && (typeof sub === 'string' ? <T v="mut">{sub}</T> : sub)}
      </View>
      {right}
      {onPress && chevron && <Text style={{ color: C.dim, fontSize: 20 }}>›</Text>}
    </Pressable>
  );
}

/** Children separated by hairlines (the prototype's .list). */
export function List({ children, style }: { children: React.ReactNode; style?: StyleProp<ViewStyle> }) {
  const kids = React.Children.toArray(children).filter(Boolean);
  return (
    <View style={style}>
      {kids.map((k, i) => <View key={i} style={i ? s.sep : undefined}>{k}</View>)}
    </View>
  );
}

/** 0–4 detector bars (kept for the Step through screen). */
export function LaughBars({ bars, size = 14 }: { bars: number; size?: number }) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 2, height: size }} accessibilityLabel={`${bars} of 4 laugh bars`}>
      {[1, 2, 3, 4].map((b) => (
        <View key={b} style={{ width: Math.max(3, size / 3.5), height: (size * b) / 4, borderRadius: 1, backgroundColor: b <= bars ? C.amber : C.border }} />
      ))}
    </View>
  );
}
export const BAR_WORDS = ['no laugh', 'chuckle', 'laugh', 'big laugh', 'applause break'];

export function Empty({ text, children }: { text: string; children?: React.ReactNode }) {
  return (
    <View style={{ alignItems: 'center', paddingVertical: 20, gap: 12 }}>
      <T v="mut" style={{ textAlign: 'center' }}>{text}</T>
      {children}
    </View>
  );
}

export function ErrorCard({ error, onRetry }: { error: string; onRetry?: () => void }) {
  return (
    <Card style={{ borderColor: C.dangerBorder, gap: 8 }}>
      <T v="semi">Can't reach Punchable</T>
      <T v="mut">{error}. Your imports wait in the outbox.</T>
      {onRetry && <Btn label="Try again" onPress={onRetry} small />}
    </Card>
  );
}

/** Eyebrow heading for a block, with an optional trailing thing. */
export function Eb({ children, right, testID }: { children: React.ReactNode; right?: React.ReactNode; testID?: string }) {
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 24 }}>
      <T v="eb" style={{ flex: 1 }}>{children}</T>
      {right}
    </View>
  );
}

/** DEV ONLY: a contract route isn't live yet and this block runs on the creative layer's demo output. */
export function FixtureBanner({ show, what }: { show: boolean; what: string }) {
  if (!show) return null;
  return (
    <View testID="fixture-banner" accessibilityRole={'note' as any} style={{ borderWidth: 1, borderStyle: 'dashed', borderColor: C.fixture, borderRadius: 8, paddingHorizontal: 10, paddingVertical: 6, backgroundColor: C.fixtureBg }}>
      <Text style={{ fontFamily: F.g7, fontSize: 11, letterSpacing: 0.5, color: C.fixtureText }}>DEV FIXTURE · {what} · route not live yet</Text>
    </View>
  );
}

// ------------------------------------------------------------------ icons
const P: Record<string, string> = {
  today: 'M3 11l9-7 9 7v9h-6v-6H9v6H3z',
  write: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
  notebook: 'M6 3h11a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6zM6 3v18M10 8h5M10 12h5',
  back: 'M15 18l-6-6 6-6',
  desk: 'M5 4h10l4 4v12H5zM9 12h6M9 16h6',
  shows: 'M12 21s-7-6.2-7-12a7 7 0 0 1 14 0c0 5.8-7 12-7 12zM12 11.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z',
  boards: 'M4 5h16v4H4zM4 11h16v4H4zM4 17h10v3H4z',
  calendar: 'M3 5h18v16H3zM3 10h18M8 3v4M16 3v4',
  notes: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
  mic: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM19 11a7 7 0 0 1-14 0M12 18v3',
  upload: 'M12 16V4M7 9l5-5 5 5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3',
  photos: 'M4 6h12v12H4zM16 10l4-2v8l-4-2M8 10l3 2-3 2z',
  lock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3',
  unlock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 7.5-2',
  search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM21 21l-5-5',
  grip: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
  up: 'M12 19V5M6 11l6-6 6 6',
  down: 'M12 5v14M6 13l6 6 6-6',
  trash: 'M5 7h14M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  list: 'M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01',
  map: 'M9 4L3 6v14l6-2 6 2 6-2V4l-6 2zM9 4v14M15 6v14',
  star: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.5 2.9 1-6.1L3.2 9.5l6.1-.9z',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  play: 'M7 4l13 8-13 8z',
  user: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0',
  // the three tabs (Mix* artboards): a spotlight, a grid of bits, a list
  tonight: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  bits: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  sets: 'M5 6h14M5 12h14M5 18h9',
  plus: 'M12 5v14M5 12h14',
  chat: 'M4 5h16v11H9l-5 4V5z',
  chevron: 'M9 6l6 6-6 6',
};
export function Icon({ name, size = 22, color = C.text, sw = 2 }: { name: keyof typeof P | string; size?: number; color?: string; sw?: number }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round">
      <Path d={P[name] || ''} />
    </Svg>
  );
}

// ------------------------------------------------------------------ layout
export const useWide = () => useWindowDimensions().width >= WIDE;

/** His face: the Google photo from the token, else his initials, else a person outline (local copy, no sign-in). */
export function Face({ size = 34 }: { size?: number }) {
  const { profile: p } = useSession();
  const [broken, setBroken] = useState(false);
  const box = { width: size, height: size, borderRadius: size / 2, backgroundColor: C.surface2, borderWidth: 1, borderColor: C.borderStrong, alignItems: 'center' as const, justifyContent: 'center' as const, overflow: 'hidden' as const };
  if (p?.picture && !broken) {
    return <Image testID="face-photo" source={{ uri: p.picture }} onError={() => setBroken(true)} accessibilityIgnoresInvertColors style={box} />;
  }
  return (
    <View style={box}>
      {p?.initials ? <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.g7, fontSize: Math.round(size * 0.38), color: C.text }}>{p.initials}</Text> : <Icon name="user" size={Math.round(size * 0.55)} color={C.mut} />}
    </View>
  );
}

/** "Sign in" for a signed-out visitor: to /login, then back to this page. */
export function SignInBtn({ testID = 'signin', full }: { testID?: string; full?: boolean }) {
  const pathname = usePathname();
  const go = () => {
    const search = ((globalThis as any).location?.search as string) || '';
    router.push(loginHref(pathname, search) as any);
  };
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel="Sign in" onPress={go}
      style={({ pressed }) => ({ minHeight: 44, minWidth: 44, paddingHorizontal: 14, borderRadius: 12, alignItems: 'center', justifyContent: 'center', backgroundColor: pressed ? C.amberMid : C.amber, alignSelf: full ? 'stretch' : 'center', marginRight: full ? 0 : 4 })}>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g7, fontSize: 15, letterSpacing: 0.3, color: C.bg }}>Sign in</Text>
    </Pressable>
  );
}

/** The avatar on a tab's top bar: opens You (profile, sign out, settings). Signed out: a Sign in button.
 *  `ask={false}` on a screen whose input bar IS Ask (Tonight): the standing decision is that Ask lives in the bar. */
export function Avatar({ ask = true }: { ask?: boolean }) {
  const sess = useSession();
  if (sess.enabled && !sess.signedIn && !sess.checking) return <SignInBtn />;
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center' }}>
      {ask && <AskButton />}
      <Pressable testID="avatar" accessibilityRole="button" accessibilityLabel="You: profile, sign out, settings" onPress={() => router.push('/profile' as any)}
        style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}>
        <Face />
      </Pressable>
    </View>
  );
}


export function IconBtn({ label, onPress, children, testID, disabled }: { label: string; onPress: () => void; children: React.ReactNode; testID?: string; disabled?: boolean }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={label} onPress={onPress} disabled={disabled}
      style={(st: any) => ({ width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center', opacity: disabled ? 0.35 : 1, backgroundColor: st.hovered ? C.surface2 : 'transparent' })}>
      {children}
    </Pressable>
  );
}

export function TextBtn({ label, onPress, testID, color = C.amber }: { label: string; onPress: () => void; testID?: string; color?: string }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" onPress={onPress} style={{ minWidth: 44, minHeight: 44, paddingHorizontal: 10, justifyContent: 'center' }}>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color }}>{label}</Text>
    </Pressable>
  );
}


/** Pull to refresh for screens that own their list (FlatList): the same control, the same colours. */
export function useRefresh(onRefresh?: () => Promise<any> | void) {
  const [refreshing, setRefreshing] = useState(false);
  const refresh = useCallback(async () => {
    if (!onRefresh) return;
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  }, [onRefresh]);
  return onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={refresh} tintColor={C.amber} colors={[C.amber]} progressBackgroundColor={C.surface} /> : undefined;
}

// ------------------------------------------------------------------ sheets
/**
 * A bottom sheet: a modal with a grabber, dismissed by the scrim, the OS back/escape, a swipe down on the grabber,
 * or the sheet's own Cancel (leading) / Done (trailing). The body sits above the keyboard (KeyboardAvoidingView),
 * so a sheet with an autofocused field keeps its buttons reachable (the iPhone blocker in the HIG audit).
 */
export function Sheet({ visible, onClose, children, label, testID }: { visible: boolean; onClose: () => void; children: React.ReactNode; label: string; testID?: string }) {
  const ins = useSafeAreaInsets();
  const { height } = useWindowDimensions();
  const y0 = useRef<number | null>(null);
  const reduce = useReduceMotion();
  const grabberTouch = {
    onTouchStart: (e: any) => { y0.current = e.nativeEvent?.pageY ?? null; },
    onTouchEnd: (e: any) => {
      const y = e.nativeEvent?.pageY;
      if (y0.current != null && typeof y === 'number' && y - y0.current > 60) onClose();
      y0.current = null;
    },
  };
  return (
    <Modal visible={visible} transparent animationType={reduce ? 'fade' : Platform.OS === 'web' ? 'fade' : 'slide'} onRequestClose={onClose}>
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={{ flex: 1 }}>
        <Pressable accessibilityLabel="Close" onPress={onClose} style={{ flex: 1, backgroundColor: C.scrim, justifyContent: 'flex-end', alignItems: 'center' }}>
          <Pressable testID={testID} accessibilityRole={'dialog' as any} aria-modal accessibilityLabel={label} onPress={(e) => e.stopPropagation()}
            style={{ width: '100%', maxWidth: 640, maxHeight: height * 0.88, backgroundColor: C.surface, borderWidth: 1, borderBottomWidth: 0, borderColor: C.border, borderTopLeftRadius: 16, borderTopRightRadius: 16 }}>
            <View {...grabberTouch} accessibilityRole="button" accessibilityLabel="Close the sheet" accessibilityHint="Swipe down or double tap" style={{ alignItems: 'center', minHeight: 28, justifyContent: 'center' }}>
              <View style={{ width: 36, height: 5, borderRadius: 3, backgroundColor: C.borderStrong }} />
            </View>
            <ScrollView contentContainerStyle={{ paddingHorizontal: 16, paddingTop: 4, paddingBottom: ins.bottom + 20 }} keyboardShouldPersistTaps="handled">
              {children}
            </ScrollView>
          </Pressable>
        </Pressable>
      </KeyboardAvoidingView>
    </Modal>
  );
}

/** A sheet's title row: Cancel LEADING, the title, Done TRAILING (HIG; Material puts the same two on a dialog). */
export function SheetHead({ title, onCancel, onDone, doneLabel = 'Done', doneDisabled, cancelLabel = 'Cancel', testID }: {
  title: string; onCancel: () => void; onDone?: () => void; doneLabel?: string; doneDisabled?: boolean; cancelLabel?: string; testID?: string;
}) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', marginBottom: 8, minHeight: 44 }}>
      <TextBtn testID={testID ? `${testID}-cancel` : undefined} label={cancelLabel} onPress={onCancel} />
      <T v="h2" numberOfLines={1} style={{ flex: 1, textAlign: 'center' }} accessibilityRole="header">{title}</T>
      {onDone ? <TextBtn testID={testID ? `${testID}-done` : undefined} label={doneLabel} onPress={onDone} color={doneDisabled ? C.dim : C.amber} /> : <View style={{ minWidth: 44 }} />}
    </View>
  );
}

export type MenuItem = { label: string; onPress: () => void; destructive?: boolean; testID?: string; disabled?: boolean; /** an SF Symbol name for the phone's native menu */ icon?: string };
/** A long-press menu as a sheet: the web and Android. iOS rows get the system context menu instead (lib/RowMenu.native). */
export function MenuSheet({ visible, title, items, onClose, testID }: { visible: boolean; title?: string; items: MenuItem[]; onClose: () => void; testID?: string }) {
  return (
    <Sheet visible={visible} onClose={onClose} label={title || 'Actions'} testID={testID}>
      {!!title && <T v="semi" numberOfLines={2} style={{ textAlign: 'center', marginBottom: 8 }}>{title}</T>}
      <View style={{ gap: 2 }}>
        {items.map((it) => (
          <Pressable key={it.label} testID={it.testID} accessibilityRole="menuitem" accessibilityState={{ disabled: !!it.disabled }} disabled={it.disabled}
            onPress={() => { onClose(); it.onPress(); }}
            style={(st: any) => ({ minHeight: 48, borderRadius: 10, paddingHorizontal: 12, justifyContent: 'center', backgroundColor: st.hovered || st.pressed ? C.surface2 : 'transparent', opacity: it.disabled ? 0.45 : 1 })}>
            <T v="semi" style={{ color: it.destructive ? C.redSoft : C.text }}>{it.label}</T>
          </Pressable>
        ))}
      </View>
      <Btn label="Cancel" v="ghost" onPress={onClose} style={{ marginTop: 10 }} />
    </Sheet>
  );
}

function Opt({ title, sub, icon, onPress, testID, badge }: { title: string; sub: string; icon: React.ReactNode; onPress: () => void; testID?: string; badge?: string | null }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" onPress={onPress}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 14, minHeight: 72, padding: 12, borderRadius: 12, borderWidth: 1, borderColor: C.border, backgroundColor: C.surface2 }}>
      {icon}
      <View style={{ flex: 1 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <T v="semi" style={{ fontSize: 17 }}>{title}</T>
          {!!badge && <View style={{ paddingHorizontal: 7, minHeight: 20, borderRadius: 10, backgroundColor: C.amber, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.bg }}>{badge}</Text></View>}
        </View>
        <T v="mut">{sub}</T>
      </View>
    </Pressable>
  );
}
const Disc = ({ bg, children, border }: { bg: string; children?: React.ReactNode; border?: boolean }) => (
  <View style={{ width: 48, height: 48, borderRadius: 24, backgroundColor: bg, alignItems: 'center', justifyContent: 'center', borderWidth: border ? 1 : 0, borderColor: C.border }}>{children}</View>
);

// ------------------------------------------------------------------ app-wide UI state
export type SnackAction = { label: string; run: () => Promise<void> | void; testID?: string };
type Snack = { id: number; msg: string; undo?: () => Promise<void> | void; action?: SnackAction };
type WSPin = { id: string; title: string; sub: string; target_s?: number } | null;
type UIState = {
  snack: (msg: string, undo?: () => Promise<void> | void, action?: SnackAction) => void;
  openCapture: () => void;
  openRecord: () => void; // alias, older screens
  quickVoiceNote: () => void;
  withMic: (then: () => void) => void;
  uploadFile: () => void;
  lastRoot: string;
  setLastRoot: (k: string) => void;
  importCount: number;
  setImportCount: (n: number) => void;
  workingSet: WSPin;
  setWorkingSet: (w: WSPin) => void;
  /** Tonight's "N to sort" count: the Tonight tab's badge on the phone (only when > 0, HIG/tab-bars). */
  toSort: number;
  setToSort: (n: number) => void;
  /** How much of the bottom a bar/dock/tabs covers on the current screen (the snackbar sits above it). */
  setPinnedHeight: (h: number) => void;
};
const Ctx = createContext<UIState | null>(null);
export const useUI = () => {
  const c = useContext(Ctx);
  if (!c) throw new Error('useUI outside UIProvider');
  return c;
};

export function UIProvider({ children }: { children: React.ReactNode }) {
  const [snack, setSnack] = useState<Snack | null>(null);
  const [sheet, setSheet] = useState<'' | 'cap' | 'mic'>('');
  const [lastRoot, setLastRoot] = useState('tonight');
  const [importCount, setImportCount] = useState(0);
  const [workingSet, setWorkingSet] = useState<WSPin>(null);
  const [toSort, setToSort] = useState(0);
  const [pinnedHeight, setPinnedHeight] = useState(0);
  const pendingMic = useRef<null | (() => void)>(null);
  const timer = useRef<any>(null);
  const wide = useWide();
  const ins = useSafeAreaInsets();

  const showSnack = useCallback((msg: string, undo?: () => Promise<void> | void, action?: SnackAction) => {
    clearTimeout(timer.current);
    const id = Date.now();
    setSnack({ id, msg, undo, action });
    timer.current = setTimeout(() => setSnack((x) => (x && x.id === id ? null : x)), undo || action ? 8000 : 4000);
  }, []);

  const withMic = useCallback((then: () => void) => {
    if (!isWeb) return then(); // the phone asks for the microphone itself (expo-audio), with the app.json wording
    if (!micSupported()) {
      showSnack('This browser can’t record audio');
      return;
    }
    if (micExplained()) return then();
    pendingMic.current = then;
    setSheet('mic');
  }, [showSnack]);

  const uploadFile = useCallback(async () => {
    setSheet('');
    const f = await pickFile();
    if (!f) return;
    showSnack(`Uploading ${f.name}…`);
    try {
      const r = await upload(f, { kind: 'upload', filename: f.name, title: f.name });
      showSnack(`Uploaded · queued for processing (${r.file})`);
    } catch (e: any) {
      showSnack(`Upload failed: ${e.message}. It stays in your outbox.`);
    }
  }, [showSnack]);

  const voiceNote = useCallback(() => {
    setSheet('');
    withMic(() => router.navigate({ pathname: '/notes', params: { talk: String(Date.now()) } } as any));
  }, [withMic]);

  const value = useMemo<UIState>(() => ({
    snack: showSnack, openCapture: () => setSheet('cap'), openRecord: () => setSheet('cap'), quickVoiceNote: voiceNote, withMic, uploadFile,
    lastRoot, setLastRoot, importCount, setImportCount, workingSet, setWorkingSet, toSort, setToSort, setPinnedHeight,
  }), [showSnack, withMic, uploadFile, voiceNote, lastRoot, importCount, workingSet, toSort]);

  return (
    <Ctx.Provider value={value}>
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        {children}
        {snack && (
          <View testID="snackbar" accessibilityRole={'status' as any} accessibilityLiveRegion="polite"
            style={{ position: 'absolute', left: 16, right: 16, bottom: wide ? 24 + (pinnedHeight > 64 ? pinnedHeight - 64 : 0) : Math.max(24, pinnedHeight + 12), zIndex: 9, alignItems: 'center' }} pointerEvents="box-none">
            <View style={{ width: '100%', maxWidth: 560, minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 4, paddingLeft: 16, paddingRight: 6, borderRadius: 8, backgroundColor: C.snackBg }}>
              <Text maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 14, color: C.snackText, paddingVertical: 8 }}>{snack.msg}</Text>
              {/* Material: one action at most. Undo wins when both are given (it is the inverse of the write). */}
              {snack.undo ? (
                <Pressable testID="snack-undo" accessibilityRole="button" onPress={async () => {
                  const u = snack.undo!;
                  setSnack(null);
                  try { await u(); showSnack('Undone'); } catch (e: any) { showSnack(`Couldn't undo: ${e.message}`); }
                }} style={{ minHeight: 44, minWidth: 44, paddingHorizontal: 8, justifyContent: 'center' }}>
                  <Text style={{ fontFamily: F.g7, fontSize: 14, color: C.snackAction }}>Undo</Text>
                </Pressable>
              ) : snack.action ? (
                <Pressable testID={snack.action.testID || 'snack-action'} accessibilityRole="button" onPress={async () => {
                  const a = snack.action!;
                  setSnack(null);
                  await a.run();
                }} style={{ minHeight: 44, minWidth: 44, paddingHorizontal: 8, justifyContent: 'center' }}>
                  <Text style={{ fontFamily: F.g7, fontSize: 14, color: C.snackAction }}>{snack.action.label}</Text>
                </Pressable>
              ) : null}
            </View>
          </View>
        )}
      </View>

      <Sheet visible={sheet === 'cap'} onClose={() => setSheet('')} label="Capture" testID="capture-sheet">
        <SheetHead title="Capture" onCancel={() => setSheet('')} testID="capture" />
        <View style={{ gap: 10 }}>
          <Opt testID="cap-import" title="Import from Photos" badge={importCount ? `${importCount} new` : null}
            sub={isWeb ? 'Pick set videos from this computer, or paste a YouTube link' : 'Your recent set videos, grouped by night'}
            icon={<Disc bg={C.amber}><Icon name="photos" color={C.bg} sw={2.2} /></Disc>}
            onPress={() => { setSheet(''); router.push('/import' as any); }} />
          <Opt testID="cap-note" title="Voice note" sub="Talk out a premise. Funny lines get pulled out and filed. (Hold ● to start now.)"
            icon={<Disc bg={C.surface} border><Icon name="mic" color={C.amber} sw={2.2} /></Disc>} onPress={voiceNote} />
          <Opt testID="cap-record" title="Record a set" sub="Optional recorder: timer, the light, marks"
            icon={<Disc bg={C.red}><View style={{ width: 18, height: 18, borderRadius: 9, backgroundColor: C.text }} /></Disc>}
            onPress={() => { setSheet(''); withMic(() => router.push('/record' as any)); }} />
          <Opt testID="cap-upload" title="Upload a file" sub="An old tape, or a set someone else filmed"
            icon={<Disc bg={C.surface2} border><Icon name="upload" /></Disc>} onPress={uploadFile} />
        </View>
      </Sheet>

      <Sheet visible={sheet === 'mic'} onClose={() => { pendingMic.current = null; setSheet(''); }} label="Microphone access" testID="mic-sheet">
        <T v="h2">Record your sets</T>
        <T style={{ marginTop: 8, marginBottom: 4 }}>Punchable needs the microphone to record sets and voice notes, so it can find each joke and measure the laughs.</T>
        <T v="mut" style={{ marginBottom: 16 }}>Audio goes only to your own server. You can turn this off anytime in Settings.</T>
        <Btn testID="mic-continue" label="Continue" v="primary" onPress={async () => {
          setSheet('');
          const ok = await requestMic();
          if (!ok) return showSnack('Microphone blocked. Allow it in the browser’s site settings.');
          setMicExplained();
          const f = pendingMic.current;
          pendingMic.current = null;
          f?.();
        }} />
        <Btn label="Not now" v="ghost" onPress={() => { pendingMic.current = null; setSheet(''); }} style={{ marginTop: 8 }} />
      </Sheet>
    </Ctx.Provider>
  );
}

/** Pads its content by the bottom inset the screen's shell measures (the system tab bar on the phone, 0 on the web).
 *  For a screen that owns its layout (`scroll={false}`) and puts a row at the very bottom. */
export function SafeBottom({ children, style }: { children: React.ReactNode; style?: StyleProp<ViewStyle> }) {
  const ins = useSafeAreaInsets();
  return <View style={[{ paddingBottom: ins.bottom }, style]}>{children}</View>;
}

// small shared glyphs
export const Dot = ({ color = C.mut, size = 8, hollow }: { color?: string; size?: number; hollow?: boolean }) => (
  <Svg width={size} height={size}><Circle cx={size / 2} cy={size / 2} r={size / 2 - (hollow ? 1 : 0)} fill={hollow ? 'none' : color} stroke={color} strokeWidth={hollow ? 1.2 : 0} /></Svg>
);
