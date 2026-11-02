// The cloud's chrome, shared by the phone (lib/Cloud.native: Skia) and the web (lib/Cloud: SVG): the loading, empty
// and failed states with the links map one tap away, the counts line, the turn buttons, the card for a selected bit,
// the cluster chips, the zoom column (+, −, Whole map). The canvas itself is the platform's; it arrives as
// `canvas(w, h)`. Spec: docs/design/SPEC-2026-11-02-cloud-3d.md.
import React, { useCallback, useEffect, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { Stack, router, useFocusEffect } from 'expo-router';
import { useBoards } from './boards';
import { slotColor } from './Galaxy';
import * as M from '@/rules/cloudMath';
import { C, F } from './theme';
import { Btn, Empty, ErrorCard, Skeleton, T, TextBtn, useBottomInset, useWide } from './ui';
import { Icon } from './Icon';
import { Screen } from './Screen';

const native = Platform.OS !== 'web';
const SLOW_AFTER_MS = 5000;

export type CloudState = {
  data: M.CloudMap | null; error: string | null; reload: () => void;
  spinning: boolean; setSpinning: (f: (v: boolean) => boolean) => void; reduce: boolean;
  focused: M.CloudNode | null; setFocus: (id: string | null) => void;
  /** fly the camera to a group (its chip, or a tap on its name) */
  fly: (c: M.CloudCluster) => void;
  /** off home (zoomed, panned or tilted): shows Whole map */
  moved: boolean;
  /** at a zoom limit: that button reads as disabled (a dead + in the dark reads as broken) */
  limit?: 'max' | 'min' | null;
  /** the buttons' and VoiceOver's camera moves: zoomIn, zoomOut, home, turnLeft, turnRight, tiltUp, tiltDown */
  command: (name: string) => void;
  onLinks: () => void;
};

/** VoiceOver's value for the canvas: "Whole map", or the zoom and the group nearest the camera's target. */
export function camValue(lz: number, tx: number, ty: number, tz: number, clusters: M.CloudCluster[], home: boolean): string {
  if (home) return 'Whole map';
  const zoom = Math.exp(lz);
  const z = zoom >= 10 ? Math.round(zoom) : Math.round(zoom * 10) / 10;
  let best: M.CloudCluster | null = null, bd = Infinity;
  for (const c of clusters) { const d = (c.cx - tx) ** 2 + (c.cy - ty) ** 2 + (c.cz - tz) ** 2; if (d < bd) { bd = d; best = c; } }
  return zoom > 1.5 && best ? `Zoom ${z}×, near ${best.label}` : `Zoom ${z}×`;
}

/** The screen in front: a pushed bit page or another tab stops the turn. */
export function useInFront(): boolean {
  const [inFront, setInFront] = useState(true);
  useFocusEffect(useCallback(() => { setInFront(true); return () => setInFront(false); }, []));
  return inFront;
}

/** True once a request has taken more than a few seconds (the API places every bit, then caches). */
export function useSlow(data: unknown, error: unknown): boolean {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (data || error) { setSlow(false); return; }
    const t = setTimeout(() => setSlow(true), SLOW_AFTER_MS);
    return () => clearTimeout(t);
  }, [data, error]);
  return slow;
}

export function countsLine(d: M.CloudMap | null): string {
  if (!d) return '';
  const missing = d.counts.placed < d.counts.jokes ? ` · ${d.counts.jokes - d.counts.placed} not placed yet` : '';
  return `${d.counts.placed} bits by meaning · ${d.clusters.length} groups${missing}`;
}

// the outline is borderStrong: on the canvas's bgDeep, `border` is ~1.5:1 and the control disappears (HIG/maps: "a thin
// stroke … to help a custom control stand out")
const round = (off = false) => ({ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, borderWidth: 1, borderColor: C.borderStrong, alignItems: 'center', justifyContent: 'center', opacity: off ? 0.4 : 1 }) as const;

export function CloudFrame({ st, canvas, onSize }: { st: CloudState; canvas: (w: number, h: number) => React.ReactNode; onSize: (w: number, h: number) => void }) {
  const wide = useWide();
  const bottom = useBottomInset();
  const boards = useBoards();
  const slow = useSlow(st.data, st.error);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const d = st.data;
  const clusterById = new Map((d?.clusters || []).map((c) => [c.id, c]));
  // SF Symbols that exist since iOS 13 (`pause`, `rotate.3d`, `link`); an unknown name would draw nothing, not throw
  const toolbar = native ? (
    <Stack.Toolbar placement="right">
      <Stack.Toolbar.Button icon={st.spinning ? 'pause' : 'rotate.3d'} accessibilityLabel={st.spinning ? 'Stop turning' : 'Turn the cloud'} onPress={() => st.setSpinning((v) => !v && !st.reduce)}>{st.spinning ? 'Still' : 'Turn'}</Stack.Toolbar.Button>
      <Stack.Toolbar.Button icon="link" accessibilityLabel="The links map instead" onPress={st.onLinks}>Links</Stack.Toolbar.Button>
    </Stack.Toolbar>
  ) : null;
  const webRight = !native ? (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, paddingRight: 8 }}>
      <TextBtn testID="cloud-turn" label={st.spinning ? 'Still' : 'Turn'} onPress={() => st.setSpinning((v) => !v && !st.reduce)} />
      <TextBtn testID="cloud-links" label="Links" onPress={st.onLinks} />
    </View>
  ) : undefined;
  const f = st.focused;
  return (
    <Screen title="Map" back backLabel="Bits" testID="cloud" scroll={false} right={webRight}>
      {toolbar}
      {st.error && !d ? <View style={{ padding: 16, gap: 10 }}><ErrorCard error={st.error} onRetry={st.reload} /><Btn small testID="cloud-links-fallback" label="Show the links map" onPress={st.onLinks} /></View> : null}
      {!d && !st.error && (
        <View style={{ padding: 16, gap: 12 }}>
          <Skeleton lines={5} height={18} testID="cloud-skeleton" />
          <T v="small" testID="cloud-wait">{slow ? 'Still placing every bit by what it says: the first time takes a few seconds, then it is instant for five minutes.' : 'Placing your bits by what they say…'}</T>
          <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
            {slow && <Btn small label="Try again" onPress={st.reload} />}
            <Btn small testID="cloud-links-fallback" label="Show the links map" onPress={st.onLinks} />
          </View>
        </View>
      )}
      {d && !d.nodes.length && (
        <Empty text="Nothing placed yet: the vectors are still being made. Try again in a minute.">
          <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap', justifyContent: 'center' }}>
            <Btn small label="Try again" onPress={st.reload} />
            <Btn small testID="cloud-links-fallback" label="Show the links map" onPress={st.onLinks} />
          </View>
        </Empty>
      )}
      {d && d.nodes.length > 0 && (
        <View style={{ flex: 1 }}>
          <View style={{ paddingHorizontal: 16, paddingTop: 8, gap: 6 }}>
            {/* VoiceOver reads the counts only: "double-tap to zoom in" would clash with its own double-tap to activate */}
            <T v="small" testID="cloud-counts" accessibilityLabel={countsLine(d)}>{countsLine(d)}{!f ? ' · drag to turn · pinch to zoom · double-tap to zoom in' : ''}</T>
          </View>
          <View style={{ flex: 1 }} onLayout={(e) => { const { width, height } = e.nativeEvent.layout; setSize({ w: width, h: height }); onSize(width, height); }}>
            {size.w > 0 && size.h > 0 && canvas(size.w, size.h)}
            {/* one container at the bottom, so the zoom column always sits above the card with nothing to measure:
                the column bottom-trailing for the thumb (+ and −: the one-handed and VoiceOver path, HIG/accessibility
                "offer onscreen ways"; then Whole map in a fixed slot, shown only off home, a text capsule because the
                expand glyph means full screen elsewhere in iOS), the selected bit's card under it */}
            <View pointerEvents="box-none" style={{ position: 'absolute', left: 12, right: 12, bottom: 12, gap: 12 }}>
              <View pointerEvents="box-none" style={{ alignSelf: 'flex-end', alignItems: 'flex-end', gap: 8 }}>
                <Pressable testID="cloud-zoom-in" accessibilityRole="button" accessibilityLabel="Zoom in" accessibilityState={{ disabled: st.limit === 'max' }} disabled={st.limit === 'max'} onPress={() => st.command('zoomIn')} style={round(st.limit === 'max')}><Icon name="plus" size={20} /></Pressable>
                <Pressable testID="cloud-zoom-out" accessibilityRole="button" accessibilityLabel="Zoom out" accessibilityState={{ disabled: st.limit === 'min' }} disabled={st.limit === 'min'} onPress={() => st.command('zoomOut')} style={round(st.limit === 'min')}><Icon name="minus" size={20} /></Pressable>
                <View pointerEvents="box-none" style={{ height: 44, justifyContent: 'center' }}>
                  {st.moved && (
                    <Pressable testID="cloud-home" accessibilityRole="button" accessibilityLabel="Whole map" onPress={() => st.command('home')}
                      style={{ height: 44, paddingHorizontal: 16, borderRadius: 22, backgroundColor: C.card, borderWidth: 1, borderColor: C.borderStrong, alignItems: 'center', justifyContent: 'center' }}>
                      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 15, color: C.text }}>Whole map</Text>
                    </Pressable>
                  )}
                </View>
              </View>
              {f && (
                <View testID="cloud-card" style={{ maxWidth: 520, padding: 12, borderRadius: 14, backgroundColor: C.card, borderWidth: 1, borderColor: C.amber, gap: 6 }}>
                  <T v="semi" numberOfLines={1}>{f.title || f.short_name}</T>
                  <T v="small" numberOfLines={1}>{f.short_name}{clusterById.get(f.cluster) ? ` · ${clusterById.get(f.cluster)!.label}` : ''} · {f.nights ? `${f.nights} night${f.nights === 1 ? '' : 's'}` : 'never tried'}</T>
                  <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
                    <Btn small v="primary" testID="cloud-open" label="Open" onPress={() => router.push(`/joke/${encodeURIComponent(f.id)}` as any)} />
                    <Btn small v="add" label="To a set" onPress={() => boards.add(f.id, f.title || f.short_name)} />
                    <Btn small v="ghost" testID="cloud-done" label="Done" onPress={() => st.setFocus(null)} />
                  </View>
                </View>
              )}
            </View>
          </View>
          {/* the frame's last row clears the home indicator and the floating tab bar (useBottomInset: build 18 drew it, and
              the card and zoom buttons in the canvas above it, under the bar on an iPhone 16); 0 on the web, whose shell pads */}
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ paddingHorizontal: 16, paddingVertical: 8, gap: 8 }} style={{ flexGrow: 0, marginBottom: bottom }}>
            {d.clusters.map((c) => (
              <Pressable key={c.id} testID={`cloud-cluster-${c.id}`} accessibilityRole="button" accessibilityLabel={`${c.label}, ${c.size} bits. Zoom to it.`} onPress={() => st.fly(c)} hitSlop={{ top: 4, bottom: 4 }}
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
