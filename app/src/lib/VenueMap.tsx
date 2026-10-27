// Web map: Leaflet + OpenStreetMap tiles, loaded only when the Map view opens (its own chunk).
// Native uses VenueMap.native.tsx (react-native-maps: Apple Maps on iOS, Google on Android).
import React, { useEffect, useRef, useState } from 'react';
import { View } from 'react-native';
import { C } from './theme';
import type { MapPin } from './mapPins';
import { T } from './ui';

export function VenueMap({ pins, onPick, height = 360, testID }: { pins: MapPin[]; onPick: (venueId: string) => void; height?: number; testID?: string }) {
  const host = useRef<any>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let cleanup: (() => void) | null = null;
    let alive = true;
    import('./leafletMount').then((m) => {
      if (!alive || !host.current) return;
      cleanup = m.mount(host.current as HTMLElement, pins, onPick);
    }).catch((e) => setErr(String(e?.message || e)));
    return () => {
      alive = false;
      cleanup?.();
    };
  }, [pins, onPick]);
  return (
    <View testID={testID} style={{ height, borderRadius: 12, overflow: 'hidden', borderWidth: 1, borderColor: C.border, backgroundColor: C.surface2 }}>
      <View ref={host} style={{ flex: 1 }} />
      {err && <T v="small" style={{ position: 'absolute', top: 8, left: 8 }}>Map couldn’t load: {err}</T>}
    </View>
  );
}
