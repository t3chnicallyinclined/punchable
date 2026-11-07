// Today (UX_SPEC 4.1): "What should I do with my comedy right now?" Slots reorder by mode:
// after a show night (until noon) · show day (from noon) · other days. ?mode= overrides (for screenshots and tests).
import React, { memo, useEffect, useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { post, undo, useApi } from '@/lib/api';
import { TodayFeed, WSView, creative, useCreative } from '@/lib/creative';
import { fmt1, fmtVs, heatWord } from '@/rules/laughs';
import { fmtShortDate, mmss } from '@/rules/format';
import { LaughHeadline } from '@/lib/metrics';
import { newTonight, setMetrics, useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { C, F } from '@/lib/theme';
import { AIBadge, AddBtn, Avatar, Btn, Card, Eb, ErrorCard, FixtureBanner, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { useWhy } from '@/lib/why';
import { MONTHS } from '@/rules/format';

type Mode = 'after_show' | 'show_day' | 'writing';
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const t0 = typeof performance !== 'undefined' ? performance.now() : 0;

function modeOf(now: Date, lastSetAt: number | null, nextShow: string | null, newVideos: number): Mode {
  const h = now.getHours();
  const today = now.toISOString().slice(0, 10);
  if (nextShow && nextShow.slice(0, 10) === today && h >= 12) return 'show_day';
  const sinceSet = lastSetAt ? now.getTime() - lastSetAt : Infinity;
  if (h < 12 && (sinceSet < 20 * 3600 * 1000 || newVideos > 0)) return 'after_show';
  return 'writing';
}

export default function Today() {
  const ui = useUI();
  const why = useWhy();
  const boards = useBoards();
  const wide = useWide();
  const q = useLocalSearchParams<{ mode?: string }>();
  const feed = useCreative<TodayFeed>('today', creative.today, { poll: 15000 });
  const old = useApi<any>('/api/today', { poll: 15000 }); // next_show, pipeline counts, inbox (existing feed)
  const inbox = useApi<{ items: any[] }>('/api/inbox');
  const sets = useSets();
  const t = feed.data;
  const wsId = t?.working_set?.working_set || ui.workingSet?.id || null;
  const ws = useCreative<WSView>(wsId ? `ws:${wsId}` : null, () => creative.workingset(wsId!));
  const latestId = t?.latest_set?.set ? `set:${t.latest_set.set.replace(/^set:/, '')}` : sets.list[0]?.id || null;
  const an = useApi<any>(latestId ? `/api/sets/${encodeURIComponent(latestId)}/analysis` : null);
  const info = latestId ? sets.index.get(latestId) : null;
  const m = useMemo(() => (an.data ? setMetrics(an.data, info) : null), [an.data, info]);

  const lastSetAt = useMemo(() => {
    const up = (inbox.data?.items || []).filter((x) => x.kind === 'set' || x.kind === 'upload').map((x) => Date.parse(x.received_at)).filter(Boolean);
    return up.length ? Math.max(...up) : null;
  }, [inbox.data]);
  const mode: Mode = (['after_show', 'show_day', 'writing'] as Mode[]).includes(q.mode as Mode)
    ? (q.mode as Mode) : modeOf(new Date(), lastSetAt, old.data?.next_show?.date || null, ui.importCount);

  const [hidden, setHidden] = useState<Record<string, number>>(() => {
    try { return JSON.parse((globalThis as any).localStorage?.getItem('pk.notNow') || '{}'); } catch { return {}; }
  });
  const notNow = (id: string) => {
    const n = { ...hidden, [id]: Date.now() + 7 * 86400000 };
    setHidden(n);
    try { (globalThis as any).localStorage?.setItem('pk.notNow', JSON.stringify(n)); } catch {}
    ui.snack('Hidden for a week', () => { setHidden(hidden); try { (globalThis as any).localStorage?.setItem('pk.notNow', JSON.stringify(hidden)); } catch {} });
  };

  useEffect(() => {
    if (t || old.data) (globalThis as any).__pkTodayRenderedMs = (typeof performance !== 'undefined' ? performance.now() : 0) - t0;
  }, [t, old.data]);

  const now = new Date();
  const header = (
    <View style={{ flexDirection: 'row', alignItems: 'flex-end', paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 8, borderBottomWidth: 1, borderBottomColor: C.border, backgroundColor: C.bg }}>
      <View style={{ flex: 1 }}>
        <T v="eb">{DAYS[now.getDay()]}, {MONTHS[now.getMonth()]} {now.getDate()}</T>
        <T v="h1">Today</T>
      </View>
      <Avatar />
    </View>
  );

  const saveLine = async (r: any) => {
    const text = (r.row.punchlines?.[r.row.punchlines.length - 1]?.text) || r.row.transcript?.slice(0, 140) || r.row.title;
    try {
      const w = await post<any>('/api/ideas', { text, title: r.row.suggested_title || r.row.title || undefined, from: { set: latestId, perf: r.row.perf, start_ms: r.row.start_ms, end_ms: r.row.end_ms, note: 'saved from Today' } });
      ui.snack('Saved to Ideas', async () => { await undo(w.indexes); });
      an.reload();
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };

  // ---------------------------------------------------------------- slots
  const banner = ui.importCount > 0 || mode === 'after_show' ? (
    <Card key="banner" testID="new-videos" style={{ borderColor: C.addBorder, gap: 10 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: C.amber }} />
        <T v="eb" style={{ color: C.amber, flex: 1 }}>{ui.importCount ? `${ui.importCount} new set ${ui.importCount === 1 ? 'video' : 'videos'}` : 'Tonight’s set'}</T>
      </View>
      <T v="mut">{ui.importCount ? 'From your Photos, grouped by night.' : 'Bring in tonight’s video. Only the audio leaves your phone.'}</T>
      <Btn testID="today-import" label="Import" v="primary" onPress={() => router.push('/import' as any)} />
    </Card>
  ) : null;

  const ls = t?.latest_set;
  const lastSet = (ls || info) ? (
    <Card key="last" testID="last-set-card" style={{ gap: 10 }}>
        <Pressable testID="latest-set" accessibilityRole="button" accessibilityLabel="Open the last set" onPress={() => latestId && router.push(`/set/${encodeURIComponent(latestId)}` as any)}
          style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}>
          <T v="eb">Last set</T><AIBadge />
          <T v="small" style={{ flex: 1, textAlign: 'right' }} numberOfLines={1}>{info ? `${info.venue.name} · ${fmtShortDate(info.show.date)}` : ls?.venue_label}  ›</T>
        </Pressable>
        <LaughHeadline testID="today-headline" value={ls?.laugh_s_per_min ?? info?.lspm} roomHeat={ls?.room_heat ?? info?.roomHeat}
          onPress={() => why.why({
            title: 'Last set · laughs', who: 'measured',
            claim: `${fmt1(ls?.laugh_s_per_min ?? info?.lspm)} seconds of laughter per minute of material`,
            reason: `${heatWord(ls?.room_heat ?? info?.roomHeat) || ''}${m ? ` · over ${mmss(m.material_s)} of material` : ''} · room heat compares it with your median set.`,
            detail: m ? <T v="small" testID="why-raw">Detector count: {an.data?.summary?.laughs ?? m.roomLaughs} laughs ({fmt1(an.data?.summary?.laughs_per_min)} raw laughs/min; a long rolling laugh can count as several).</T> : null,
            whoLine: 'Measured automatically from the tape · not hand-checked yet.',
          })} />
        {(ls?.best || info?.best) && (() => {
          const bid = ls?.best?.joke_id || info?.best?.id;
          const br = m?.rows.find((r) => r.row.joke?.id === bid);
          return (
            <View style={s.row}>
              <T v="small" style={{ width: 44 }}>Best</T>
              <T v="semi" style={{ flex: 1 }} numberOfLines={1}>{ls?.best?.title || info?.best?.title}</T>
              {br?.comparable && <T v="small">{fmtVs(br.vs)}{br.row.laughs?.laugh_s ? ` · ${Math.round(br.row.laughs.laugh_s)} s laughs` : ''}</T>}
            </View>
          );
        })()}
        {m && newTonight(m).slice(0, 1).map((r) => (
          <View key={r.idx} style={[s.row, { alignItems: 'flex-start' }]} testID="today-new">
            <T v="small" style={{ width: 44, paddingTop: 2 }}>New</T>
            <View style={{ flex: 1 }}>
              <T numberOfLines={2}>“{r.row.punchlines?.[r.row.punchlines.length - 1]?.text || r.row.title}”</T>
              {r.comparable && <T v="small">{fmtVs(r.vs)}</T>}
            </View>
            {r.row.saved_as ? <T v="small" style={{ color: C.amber }}>Saved</T> : <Btn testID="today-save-new" small label="Save" onPress={() => saveLine(r)} />}
          </View>
        ))}
        {m?.quiet && m.quiet.s >= 60 && <T v="small">Quiet {mmss(m.quiet.s)} at {mmss(m.quiet.at_ms / 1000)}</T>}
    </Card>
  ) : null;

  const st = t?.working_set || ws.data?.status;
  const wsCard = st ? (
    <Pressable key="ws" testID="today-ws" accessibilityRole="button" onPress={() => router.push(`/workingset/${encodeURIComponent(st.working_set)}` as any)}>
      <Card style={{ gap: 6, borderColor: mode === 'show_day' ? C.addBorder : C.border }}>
        <View style={s.row}>
          <T v="eb" style={{ color: C.amber, flex: 1 }}>{mode === 'show_day' ? 'Tonight · ' : ''}★ Working set v{st.version}</T>
          <T v="num" style={{ fontSize: 14 }}>{ws.data ? `${mmss(ws.data.version.bits.reduce((a, b) => a + b.target_s, 0))} / ${mmss(st.target_s)}` : mmss(st.target_s)}</T>
        </View>
        <T v="semi" numberOfLines={1}>{st.title}</T>
        {st.last_night && (
          <T v="mut">last night {fmt1(st.last_night.ws_laugh_s_per_min)} laugh-s/min {st.vs_previous === 'up' ? '▲' : st.vs_previous === 'down' ? '▼' : ''} · {st.last_night.bits_performed} of {st.last_night.bits_planned} bits</T>
        )}
        <T v="mut">{(t?.try_tonight || ws.data?.try_tonight || []).length} things to try · {Math.round(((ws.data?.perfected.locked_share ?? 0)) * 100)}% perfected  ›</T>
      </Card>
    </Pressable>
  ) : (
    <Card key="ws" testID="today-ws-empty" style={{ gap: 8 }}>
      <T v="eb">Working set</T>
      <T v="mut">Promote a board to your Working set and it becomes the spine of Today.</T>
      <Btn small label="Open Boards" onPress={() => router.navigate('/boards' as any)} />
    </Card>
  );

  const tries = (t?.try_tonight || []).slice(0, 3);
  const tryCard = mode === 'show_day' && tries.length ? (
    <Card key="try" testID="today-try" style={{ gap: 8 }}>
      <T v="eb">3 to try tonight</T>
      {tries.map((x, i) => <T key={i} numberOfLines={2}>☐ {x.text}</T>)}
    </Card>
  ) : null;

  const worst = ws.data?.bits.filter((b) => b.last_night?.vs_room != null && !b.locked).sort((a, b) => (a.last_night!.vs_room! - b.last_night!.vs_room!))[0];
  const writeNext = worst ? (
    <Card key="write" testID="write-next" style={{ gap: 6 }}>
      <T v="eb">Write next</T>
      <View style={s.row}>
        <Pressable style={{ flex: 1 }} onPress={() => router.push(`/joke/${encodeURIComponent(worst.joke_id)}` as any)} accessibilityRole="button">
          <T v="semi">{worst.title}</T>
          <T v="mut">{worst.trend === 'down' ? 'slipping' : 'weakest'} in the Working set: {fmtVs(worst.last_night!.vs_room)} last night</T>
        </Pressable>
        <AddBtn testID="write-next-add" onPress={() => boards.add(worst.joke_id, worst.title)} onLongPress={() => boards.pick(worst.joke_id, worst.title)} />
      </View>
    </Card>
  ) : null;

  const gem = (t?.serendipity || []).find((g) => !(hidden[g.joke_id] > Date.now()));
  const insight = gem ? (
    <Card key="insight" testID="insight" style={{ gap: 8 }}>
      <View style={s.row}><T v="eb" style={{ flex: 1 }}>{gem.kind === 'gem' ? 'A buried gem' : 'Never on tape'}</T><AIBadge /></View>
      <Pressable accessibilityRole="button" onPress={() => router.push(`/joke/${encodeURIComponent(gem.joke_id)}` as any)}>
        <T v="semi">{gem.title} <Text style={{ color: C.mut, fontFamily: F.body }}>({gem.stage})</Text></T>
        <T v="mut" numberOfLines={3}>{gem.why}</T>
      </Pressable>
      <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
        <AddBtn testID="insight-add" onPress={() => boards.add(gem.joke_id, gem.title)} onLongPress={() => boards.pick(gem.joke_id, gem.title)} />
        <TextBtn testID="insight-not-now" label="Not now" color={C.mut} onPress={() => notNow(gem.joke_id)} />
      </View>
    </Card>
  ) : null;

  const slots: (React.ReactElement | null)[] =
    mode === 'after_show' ? [banner, lastSet, wsCard, insight]
      : mode === 'show_day' ? [wsCard, tryCard, lastSet]
        : [wsCard, writeNext, insight, lastSet];

  const pipeline = old.data?.pipeline || {};
  const footer = (
    <View testID="today-footer" style={{ gap: 4, paddingTop: 4 }}>
      {t?.minutes && <T v="small">{fmt1(t.minutes.working_plus_testing_min)} min of stage-tested material ({t.minutes.n_working} working, {t.minutes.n_testing} testing)</T>}
      {!!Object.keys(pipeline).length && <T v="small">{['working', 'testing', 'to_try', 'idea'].map((k) => `${pipeline[k] ?? 0} ${k.replace('_', ' ')}`).join(' · ')}</T>}
      <T v="small">{old.stale || feed.stale ? 'Showing what this phone saved' : 'Up to date'}</T>
    </View>
  );

  const err = !t && !old.data && (feed.error || old.error);
  const visible = slots.filter(Boolean) as React.ReactElement[];
  return (
    <Screen title="Today" header={header} testID={`today-${mode}`}>
      <FixtureBanner show={feed.fixture} what="GET /api/today (today.build)" />
      {err ? <ErrorCard error={String(err)} onRetry={() => { feed.reload(); old.reload(); }} /> : null}
      {wide ? (
        <View style={{ flexDirection: 'row', gap: 16, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, gap: 16 }}>{visible.slice(0, 2)}</View>
          <View style={{ flex: 2, gap: 16 }}>{visible.slice(2)}</View>
        </View>
      ) : visible}
      {footer}
      <SlotOrder slots={visible.map((x) => String(x.key))} />
    </Screen>
  );
}

/** The slot order as data, for the test (P0-10). */
const SlotOrder = memo(function SlotOrder({ slots }: { slots: string[] }) {
  return <View testID={`slot-order:${slots.join(',')}`} style={{ height: 0, overflow: 'hidden' }} />;
});
