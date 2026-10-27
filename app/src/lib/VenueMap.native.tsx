// iOS / Android map: react-native-maps (Apple Maps on iOS; Google Maps on Android needs a key in app.json).
import React, { useMemo } from 'react';
import { View } from 'react-native';
import MapView, { Marker } from 'react-native-maps';
import { C } from './theme';
import type { MapPin } from './mapPins';

export function VenueMap({ pins, onPick, height = 360, testID }: { pins: MapPin[]; onPick: (venueId: string) => void; height?: number; testID?: string }) {
  const placed = pins.filter((p) => p.lat != null && p.lng != null);
  const region = useMemo(() => {
    if (!placed.length) return { latitude: 40.73, longitude: -73.99, latitudeDelta: 0.2, longitudeDelta: 0.2 };
    const lats = placed.map((p) => p.lat!);
    const lngs = placed.map((p) => p.lng!);
    const pad = 1.4;
    return {
      latitude: (Math.min(...lats) + Math.max(...lats)) / 2, longitude: (Math.min(...lngs) + Math.max(...lngs)) / 2,
      latitudeDelta: Math.max(0.05, (Math.max(...lats) - Math.min(...lats)) * pad), longitudeDelta: Math.max(0.05, (Math.max(...lngs) - Math.min(...lngs)) * pad),
    };
  }, [placed]);
  return (
    <View testID={testID} style={{ height, borderRadius: 12, overflow: 'hidden', borderWidth: 1, borderColor: C.border }}>
      <MapView style={{ flex: 1 }} initialRegion={region} userInterfaceStyle="dark">
        {placed.map((p) => (
          <Marker key={p.venue_id} coordinate={{ latitude: p.lat!, longitude: p.lng! }} title={p.name} description={`${p.n_sets} ${p.n_sets === 1 ? 'set' : 'sets'} · ${p.heatWord}`}
            onCalloutPress={() => onPick(p.venue_id)} onPress={() => onPick(p.venue_id)}>
            <View style={{ width: p.radius * 2, height: p.radius * 2, borderRadius: p.radius, backgroundColor: p.fill, opacity: Math.max(0.5, p.fillOpacity), borderWidth: 2, borderColor: C.bg }} />
          </Marker>
        ))}
      </MapView>
    </View>
  );
}
