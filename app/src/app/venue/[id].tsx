// Venue page (UX_SPEC 4.7): every set there by date, what works in this room (with n), the weakest, and your
// average there vs everywhere. Rename or place the pin: one write each (PATCH /api/venues/{id}).
import React, { useEffect, useMemo, useState } from 'react';
import { TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { VenuesView, creative, useCreative } from '@/lib/creative';
import { fmt1, fmtShortDate, fmtVs, heatWord, roomComparable } from '@/lib/laughs';
import { HeatDot } from '@/lib/metrics';
import { useSets } from '@/lib/sets';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Card, Empty, ErrorCard, FixtureBanner, List, ListRow, Screen, Sheet, T, s, useUI } from '@/lib/ui';

export default function VenuePage() {
  const p = useLocalSearchParams<{ id: string; place?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const v = useCreative<VenuesView>('venues', creative.venues);
  const sets = useSets();
  const venue = v.data?.venues.find((x) => x.venue_id === id);
  const amb = v.data?.ambiguous.find((x) => x.venue_id === id);
  const [edit, setEdit] = useState(p.place === '1');
  const [name, setName] = useState('');
  const [lat, setLat] = useState('');
  const [lon, setLon] = useState('');
  useEffect(() => {
    if (venue) { setName(venue.name); setLat(venue.geo?.lat != null ? String(venue.geo.lat) : ''); setLon(venue.geo?.lon != null ? String(venue.geo.lon) : ''); }
  }, [venue?.venue_id]); // eslint-disable-line react-hooks/exhaustive-deps
  const median = useMemo(() => {
    const xs = sets.list.filter((x) => !x.unreliable && x.lspm != null).map((x) => x.lspm!).sort((a, b) => a - b);
    return xs.length ? xs[Math.floor(xs.length / 2)] : null;
  }, [sets.list]);

  if (v.error && !v.data) return <Screen title="Venue" back><ErrorCard error={v.error} onRetry={v.reload} /></Screen>;
  if (!venue) return <Screen title="Venue" back><Empty text={v.loading ? 'Loading…' : 'No such venue.'} /></Screen>;
  const quiet = venue.sets.every((x) => { const info = sets.index.get(`set:${x.set.replace(/^set:/, '')}`); return info?.unreliable || !roomComparable(info?.roomLaughs); });
  const input = { minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 } as const;
  return (
    <Screen title="Venue" back testID="venue">
      <FixtureBanner show={v.fixture} what="GET /api/venues" />
      <View style={{ gap: 4 }}>
        <View style={s.row}><T v="eb" style={{ flex: 1 }}>{venue.raw}</T>{amb && <AIBadge />}</View>
        <T v="h1" testID="venue-name">{venue.name}</T>
        {amb && <T v="small">The AI isn’t sure about this place: {amb.reasons.join('; ')}.</T>}
        <Btn testID="venue-edit" small v="ghost" label="Rename or place on map" onPress={() => setEdit(true)} style={{ alignSelf: 'flex-start' }} />
      </View>
      <Card style={{ gap: 6 }} testID="venue-avg">
        <T v="eb">You here</T>
        <View style={s.row}><T v="num" style={{ fontSize: 26 }}>{fmt1(venue.avg_laugh_s_per_min)}</T><T v="mut">laugh-s/min avg · {venue.n_sets} {venue.n_sets === 1 ? 'set' : 'sets'}</T><HeatDot heat={venue.avg_room_heat} /></View>
        <T v="mut">{median != null ? `Your median everywhere: ${fmt1(median)} (${sets.list.filter((x) => !x.unreliable).length} sets)` : ''} · {heatWord(venue.avg_room_heat)}</T>
      </Card>
      <Card style={{ gap: 4 }} testID="venue-sets">
        <T v="eb">Sets here</T>
        <List>
          {venue.sets.map((x) => {
            const sid = `set:${x.set.replace(/^set:/, '')}`;
            const info = sets.index.get(sid);
            return <ListRow key={x.set} testID={`venue-set-${x.set}`} title={`${fmtShortDate(info?.show.date || x.date)} · ${x.title}`} sub={info?.unreliable ? '⚠ audio unreliable' : `${fmt1(x.laugh_s_per_min)} laugh-s/min · ${heatWord(x.room_heat)}`} onPress={() => router.push(`/set/${encodeURIComponent(sid)}` as any)} />;
          })}
        </List>
      </Card>
      <Card style={{ gap: 6 }} testID="works-here">
        <T v="eb">What works in this room</T>
        {quiet ? <T v="mut">The room was too quiet to compare bits here.</T> : venue.works_here.length ? venue.works_here.map((w) => (
          <ListRow key={w.joke_id} title={w.title.split(' (')[0]} sub={`${fmtVs(w.here_vs_room ?? w.vs_room_here)} here vs ${fmtVs(w.elsewhere_vs_room)} elsewhere · n ${w.n_here} here, ${w.n_elsewhere ?? '?'} elsewhere${w.low_n ? ' · early read' : ''}`}
            onPress={() => router.push(`/joke/${encodeURIComponent(w.joke_id)}` as any)} />
        )) : <T v="mut">Nothing stands out here yet.</T>}
        {!quiet && !!venue.worst_jokes?.length && <>
          <T v="eb" style={{ marginTop: 8 }}>Weakest here</T>
          {venue.worst_jokes.slice(0, 3).map((w: any) => <ListRow key={w.joke_id} title={w.title.split(' (')[0]} sub={`${fmtVs(Math.min(3, w.vs_room))} · ${fmtShortDate(w.date)}`} onPress={() => router.push(`/joke/${encodeURIComponent(w.joke_id)}` as any)} />)}
        </>}
        <T v="small">{venue.note}</T>
      </Card>
      <Sheet visible={edit} onClose={() => setEdit(false)} label="Rename or place" testID="venue-edit-sheet">
        <T v="h2">Rename or place {venue.name}</T>
        <View style={{ gap: 8, marginTop: 8 }}>
          <T v="small">Name</T>
          <TextInput testID="venue-name-input" value={name} onChangeText={setName} style={input} />
          <Btn testID="venue-rename" small label="Save name" disabled={!name.trim() || name === venue.name} onPress={async () => {
            setEdit(false);
            const old = venue.name;
            await creative.patchVenue(id, { name: name.trim() });
            v.reload();
            ui.snack(`Renamed to ${name.trim()}`, async () => { await creative.patchVenue(id, { name: old }); v.reload(); });
          }} />
          <T v="small">Place (latitude, longitude)</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <TextInput testID="venue-lat" value={lat} onChangeText={setLat} placeholder="40.73" placeholderTextColor={C.dim} style={[input, { flex: 1 }]} keyboardType="numeric" />
            <TextInput testID="venue-lon" value={lon} onChangeText={setLon} placeholder="-73.99" placeholderTextColor={C.dim} style={[input, { flex: 1 }]} keyboardType="numeric" />
          </View>
          <Btn testID="venue-place" small label="Save place" disabled={!isFinite(Number(lat)) || !isFinite(Number(lon)) || !lat || !lon} onPress={async () => {
            setEdit(false);
            await creative.patchVenue(id, { lat: Number(lat), lon: Number(lon) });
            v.reload();
            ui.snack(`${venue.name} placed on the map`);
          }} />
        </View>
      </Sheet>
    </Screen>
  );
}
