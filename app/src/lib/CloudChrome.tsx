// The cloud's chrome, shared by the phone (lib/Cloud.native: Skia) and the web (lib/Cloud: SVG): the loading, empty
// and failed states with the links map one tap away, the counts line, the turn buttons, the card for a selected bit,
// the cluster chips. The canvas itself is the platform's; it arrives as `canvas(w, h)`.
import React, { useCallback, useEffect, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { Stack, router, useFocusEffect } from 'expo-router';
import { useBoards } from './boards';
import { slotColor } from './Galaxy';
import * as M from './cloudMath';
import { C, F } from './theme';
import { Btn, Empty, ErrorCard, Skeleton, T, TextBtn, useWide } from './ui';
import { Screen } from './Screen';

const native = Platform.OS !== 'web';
const SLOW_AFTER_MS = 5000;

export type CloudState = {
  data: M.CloudMap | null; error: string | null; reload: () => void;
  spinning: boolean; setSpinning: (f: (v: boolean) => boolean) => void; reduce: boolean;
  focused: M.CloudNode | null; setFocus: (id: string | null) => void;
  face: (c: M.CloudCluster) => void; turnBy: (rad: number) => void;
  onLinks: () => void;
};

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

export function CloudFrame({ st, canvas, onSize }: { st: CloudState; canvas: (w: number, h: number) => React.ReactNode; onSize: (w: number, h: number) => void }) {
  const wide = useWide();
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
            <T v="small" testID="cloud-counts">{countsLine(d)}{!f ? ' · near = says the same kind of thing · tap a dot' : ''}</T>
          </View>
          <View style={{ flex: 1 }} onLayout={(e) => { const { width, height } = e.nativeEvent.layout; setSize({ w: width, h: height }); onSize(width, height); }}>
            {size.w > 0 && size.h > 0 && canvas(size.w, size.h)}
            <View style={{ position: 'absolute', right: 12, top: 12, gap: 6 }}>
              <Pressable testID="cloud-left" accessibilityRole="button" accessibilityLabel="Turn left" onPress={() => st.turnBy(-Math.PI / 12)} style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.text, fontSize: 18 }}>⟲</Text></Pressable>
              <Pressable testID="cloud-right" accessibilityRole="button" accessibilityLabel="Turn right" onPress={() => st.turnBy(Math.PI / 12)} style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.text, fontSize: 18 }}>⟳</Text></Pressable>
            </View>
            {f && (
              <View testID="cloud-card" style={{ position: 'absolute', left: 12, right: 12, bottom: 12, maxWidth: 520, padding: 12, borderRadius: 14, backgroundColor: C.card, borderWidth: 1, borderColor: C.amber, gap: 6 }}>
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
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ paddingHorizontal: 16, paddingVertical: 8, gap: 8 }} style={{ flexGrow: 0 }}>
            {d.clusters.map((c) => (
              <Pressable key={c.id} testID={`cloud-cluster-${c.id}`} accessibilityRole="button" accessibilityLabel={`${c.label}, ${c.size} bits. Turn it to the front.`} onPress={() => st.face(c)}
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
