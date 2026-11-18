// Shows (UX_SPEC 4.7): how have my shows gone, where, and what works in which room. List | Calendar | Map.
import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { GeoJSON, creative, useCreative } from '@/lib/creative';
import { fmt1, heatWord } from '@/rules/laughs';
import { fmtShortDate, mmss, MONTHS_LONG } from '@/rules/format';
import { pinsOf } from '@/lib/mapPins';
import { HeatDot } from '@/lib/metrics';
import { SetInfo, useSets } from '@/lib/sets';
import { VenueMap } from '@/lib/VenueMap';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Card, Empty, ErrorCard, FixtureBanner, List, Seg, Sheet, T, s, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type V = 'list' | 'calendar' | 'map';

function ShowRow({ x }: { x: SetInfo }) {
  return (
    <View testID={`show-${x.id}`} style={{ paddingVertical: 10, gap: 4 }}>
      <Pressable accessibilityRole="button" accessibilityLabel={`${x.venue.name}, ${fmtShortDate(x.show.date)}. Open set`} onPress={() => router.push(`/set/${encodeURIComponent(x.id)}` as any)} style={{ gap: 4, minHeight: 44 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <T v="num" style={{ fontSize: 14, width: 78 }} testID={`show-date-${x.id}`}>{fmtShortDate(x.show.date)}</T>
          {x.show.inferred && <AIBadge />}
          <T v="semi" numberOfLines={1} style={{ flex: 1 }}>{x.venue.name}</T>
          {x.venue.guessed && <AIBadge />}
          <T v="small">{mmss(x.card.duration_s)}</T>
        </View>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 84 }}>
          {x.unreliable ? (
            <T v="small" testID={`show-unreliable-${x.id}`}>⚠ audio unreliable · not in trends</T>
          ) : (
            <>
              <T v="small" style={{ color: C.text2 }}>{fmt1(x.lspm)} laugh-s/min</T>
              <HeatDot heat={x.roomHeat} /><T v="small">{heatWord(x.roomHeat)?.replace('about your ', '') || ''}</T>
              {x.best && <T v="small" numberOfLines={1} style={{ flex: 1 }}>· {x.best.title.split(' (')[0]}</T>}
            </>
          )}
        </View>
      </Pressable>
      {x.venueId && <Pressable testID={`venue-link-${x.id}`} accessibilityRole="link" onPress={() => router.push(`/venue/${encodeURIComponent(x.venueId!)}` as any)} style={{ paddingLeft: 84, minHeight: 28, justifyContent: 'center' }}>
        <T v="small" style={{ color: C.amber }}>What works at {x.venue.name.replace(/\?$/, '')} ›</T>
      </Pressable>}
    </View>
  );
}

export default function Shows() {
  const q = useLocalSearchParams<{ view?: string }>();
  const [view, setView] = useState<V>(q.view === 'map' ? 'map' : 'list'); // the calendar is gone (SIMPLE.md): the list is the calendar
  const sets = useSets();
  const wide = useWide();
  return (
    <Screen title="Shows" testID="shows">
      <FixtureBanner show={sets.fixture} what="GET /api/venues" />
      <Seg testID="shows-view" items={[['list', 'List'], ['map', 'Map']]} value={view} onChange={setView} />
      {sets.error && !sets.list.length ? <ErrorCard error={sets.error} onRetry={sets.reload} /> : null}
      {!sets.list.length && !sets.error && <Empty text={sets.loading ? 'Loading your shows…' : 'Import a set to start your show history.'} />}
      {view === 'list' && !!sets.list.length && (
        <Card testID="show-list" style={{ paddingVertical: 4 }}>
          <T v="eb" style={{ paddingTop: 8 }}>Past</T>
          <List>{sets.list.map((x) => <ShowRow key={x.id} x={x} />)}</List>
        </Card>
      )}
      {view === 'calendar' && !!sets.list.length && <CalendarView list={sets.list} wide={wide} />}
      {view === 'map' && <MapPane />}
    </Screen>
  );
}

function CalendarView({ list, wide }: { list: SetInfo[]; wide: boolean }) {
  const months = useMemo(() => [...new Set(list.map((x) => (x.show.date || '').slice(0, 7)))].sort().reverse(), [list]);
  const [mi, setMi] = useState(0);
  const [pick, setPick] = useState<SetInfo | null>(list[0] || null);
  const ym = months[mi] || '';
  const [y, m] = ym.split('-').map(Number);
  const first = new Date(y, (m || 1) - 1, 1).getDay();
  const days = new Date(y, m || 1, 0).getDate();
  const byDay = new Map<number, SetInfo[]>();
  for (const x of list) if ((x.show.date || '').startsWith(ym)) {
    const d = Number(x.show.date!.slice(8, 10));
    byDay.set(d, [...(byDay.get(d) || []), x]);
  }
  const cells: (number | null)[] = [...Array(first).fill(null), ...Array.from({ length: days }, (_, i) => i + 1)];
  return (
    <View style={{ gap: 12 }}>
      <Card testID="month-grid" style={{ gap: 8 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center' }}>
          <Btn small v="ghost" label="‹" accessibilityLabel="Earlier show month" disabled={mi >= months.length - 1} onPress={() => setMi(mi + 1)} />
          <T v="h2" style={{ flex: 1, textAlign: 'center' }} testID="month-title">{MONTHS_LONG[(m || 1) - 1]} {y}</T>
          <Btn small v="ghost" label="›" accessibilityLabel="Later show month" disabled={mi <= 0} onPress={() => setMi(mi - 1)} />
        </View>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap' }}>
          {['S', 'M', 'T', 'W', 'T', 'F', 'S'].map((d, i) => <T key={i} v="small" style={{ width: `${100 / 7}%`, textAlign: 'center' }}>{d}</T>)}
          {cells.map((d, i) => {
            const shows = d ? byDay.get(d) : null;
            return (
              <Pressable key={i} disabled={!shows} onPress={() => shows && setPick(shows[0])} accessibilityRole={shows ? 'button' : undefined}
                accessibilityLabel={shows ? `${d}: ${shows.map((x) => x.venue.name).join(', ')}` : undefined} testID={shows ? `day-${d}` : undefined}
                style={{ width: `${100 / 7}%`, height: 44, alignItems: 'center', justifyContent: 'center', borderRadius: 8, backgroundColor: shows && pick && shows.includes(pick) ? C.amberSoft : 'transparent' }}>
                {d && <Text style={{ fontFamily: F.g5, fontSize: 14, color: shows ? C.text : C.dim }}>{d}</Text>}
                {shows && <HeatDot heat={shows[0].unreliable ? null : shows[0].roomHeat} />}
              </Pressable>
            );
          })}
        </View>
        <T v="small">○ quiet room · ◐ about your usual · ● hot room</T>
      </Card>
      {pick && (
        <Card testID="show-card" style={{ gap: 6 }}>
          <T v="eb">{fmtShortDate(pick.show.date)} · {pick.venue.name}</T>
          <T v="semi">{pick.title}</T>
          {pick.unreliable ? <T v="mut">⚠ audio unreliable · not in trends</T> : <T>{fmt1(pick.lspm)} laugh-s/min · {heatWord(pick.roomHeat)}</T>}
          {pick.best && <T v="mut">best: {pick.best.title.split(' (')[0]}</T>}
          <Btn testID="open-set" label="Open set" v="primary" onPress={() => router.push(`/set/${encodeURIComponent(pick.id)}` as any)} />
        </Card>
      )}
      <List>{months.map((mm, i) => <Pressable key={mm} onPress={() => setMi(i)} style={{ minHeight: 44, justifyContent: 'center' }}><T v="mut">{MONTHS_LONG[Number(mm.slice(5, 7)) - 1]} {mm.slice(0, 4)} · {list.filter((x) => (x.show.date || '').startsWith(mm)).length} show</T></Pressable>)}</List>
    </View>
  );
}

function MapPane() {
  const g = useCreative<GeoJSON>('venues.geojson', creative.geojson);
  const v = useCreative<any>('venues', creative.venues);
  const pins = useMemo(() => pinsOf(g.data), [g.data]);
  const [sheet, setSheet] = useState<string | null>(null);
  const onPick = useCallback((id: string) => setSheet(id), []);
  const venue = v.data?.venues?.find((x: any) => x.venue_id === sheet);
  const unplaced = pins.filter((p) => p.lat == null);
  return (
    <View style={{ gap: 12 }} testID="map-pane">
      <FixtureBanner show={g.fixture} what="GET /api/venues.geojson" />
      <VenueMap testID="venue-map" pins={pins} onPick={onPick} />
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 12 }}>
        <T v="small">Pin colour = how the room laughed: grey quiet · dark amber usual · bright amber hot. Bigger pin = more sets.</T>
      </View>
      {!!unplaced.length && (
        <Card testID="unplaced" style={{ gap: 6 }}>
          <T v="semi">{unplaced.length} venues without a place</T>
          <T v="mut">{unplaced.map((p) => p.name).join(', ')}</T>
          {unplaced.map((p) => <Btn key={p.venue_id} small v="ghost" label={`Place ${p.name}`} onPress={() => router.push(`/venue/${encodeURIComponent(p.venue_id)}?place=1` as any)} />)}
        </Card>
      )}
      <T v="small" testID="tile-privacy">Map tiles come from OpenStreetMap (Apple Maps on iPhone); they see which area you’re looking at.</T>
      <Sheet visible={!!sheet} onClose={() => setSheet(null)} label="Venue" testID="venue-sheet">
        {venue && (
          <View style={{ gap: 8 }}>
            <T v="h2">{venue.name}</T>
            <T v="mut">{venue.n_sets} {venue.n_sets === 1 ? 'set' : 'sets'} · avg {fmt1(venue.avg_laugh_s_per_min)} laugh-s/min · {heatWord(venue.avg_room_heat)}</T>
            {(venue.works_here || []).slice(0, 3).map((w: any) => <T key={w.joke_id}>{w.title.split(' (')[0]} · better here (n {w.n_here})</T>)}
            <Btn testID="venue-open" label="Open venue" v="primary" onPress={() => { setSheet(null); router.push(`/venue/${encodeURIComponent(venue.venue_id)}` as any); }} />
          </View>
        )}
      </Sheet>
    </View>
  );
}
