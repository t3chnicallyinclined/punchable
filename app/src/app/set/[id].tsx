// Set (UX_SPEC 4.4): how that set went, bit by bit, and what's new.
// Summary (laugh-s/min + room heat in words, % laughing, rolling laughs/min, longest laugh, longest quiet) ·
// density timeline · vs Working set · New tonight · best clips · rows in order (rating word derived from × room) ·
// "This was…" sheet instead of resting Confirm / Reject buttons.
import React, { memo, useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { decide, post, setRow, undo, useApi } from '@/lib/api';
import { Alignment, ClipsView, creative, useCreative } from '@/lib/creative';
import { BAR_WORDS, RATINGS, Rating, fmt1, fmtDur, fmtPct, fmtSec, fmtShortDate, fmtVs, heatWord, nonBitWord, ratingFromVsRoom } from '@/lib/laughs';
import { DensityStrip, LaughHeadline, RatingTag, ReceptionLegend, ReceptionMeter, VsRoomBar } from '@/lib/metrics';
import { RowM, editSet, newTonight, setMetrics, useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { playSegment, stopPlayback } from '@/lib/media';
import { C, F } from '@/lib/theme';
import { AIBadge, AddBtn, Btn, Card, Empty, ErrorCard, FixtureBanner, List, ListRow, Screen, Sheet, T, TextBtn, Yours, s, useUI, useWide } from '@/lib/ui';
import { WhyLine, useWhy } from '@/lib/why';

const mmss = (ms: number) => fmtDur(ms / 1000);

export default function SetScreen() {
  const p = useLocalSearchParams<{ id: string; row?: string; at?: string; play?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const why = useWhy();
  const wide = useWide();
  const an = useApi<any>(`/api/sets/${encodeURIComponent(id)}/analysis`, { poll: 15000 });
  const sets = useSets();
  const info = sets.index.get(id) || null;
  const wsId = ui.workingSet?.id || null;
  const nights = useCreative<{ nights: Alignment[] }>(wsId ? `nights:${wsId}` : null, () => creative.nights(wsId!));
  const clips = useApi<ClipsView>(`/api/sets/${encodeURIComponent(id)}/clips`, { cache: false });
  const m = useMemo(() => (an.data ? setMetrics(an.data, info) : null), [an.data, info]);
  const align = nights.data?.nights.find((n) => `set:${n.set.replace(/^set:/, '')}` === id && n.totals.bits_performed > 0) || null;
  const [open, setOpen] = useState<Record<number, boolean>>({});
  const [thisWas, setThisWas] = useState<RowM | null>(null);
  const [rateRow, setRateRow] = useState<RowM | null>(null);
  const [editVenue, setEditVenue] = useState(false);
  const scroll = useRef<any>(null);
  const jumped = useRef(false);

  const jump = (idx: number, play = false) => {
    setOpen((o) => ({ ...o, [idx]: true }));
    setTimeout(() => {
      if (Platform.OS === 'web') (globalThis as any).document?.querySelector(`[data-testid="row-${idx}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }, 60);
    if (play) {
      const r = m?.rows.find((x) => x.idx === idx);
      if (r && an.data?.audio?.opus) playSegment(an.data.audio.opus, r.row.play?.start_ms ?? Math.max(0, r.row.start_ms - 4000), r.row.play?.end_ms ?? r.row.end_ms, `row-${idx}`).catch(() => {});
    }
  };
  useEffect(() => {
    if (!m || jumped.current) return;
    const at = p.at ? Number(p.at) : null;
    const idx = p.row ? Number(p.row) : at != null ? m.rows.find((r) => r.row.start_ms <= at + 1500 && at <= r.row.end_ms)?.idx : null;
    if (idx != null) {
      jumped.current = true;
      jump(idx, p.play === '1');
    }
  }, [m]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => stopPlayback(), []);

  const act = async (fn: () => Promise<{ indexes: number[] }[]>, msg: string) => {
    try {
      const ws = await fn();
      ui.snack(msg, async () => { await undo(ws.flatMap((w) => w.indexes).sort((a, b) => b - a)); an.reload(); });
      an.reload();
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };

  if (an.error && !an.data) return <Screen title="Set" back><ErrorCard error={an.error} onRetry={an.reload} /></Screen>;
  if (!an.data || !m) return <Screen title="Set" back><Empty text="Loading the set…" /></Screen>;
  const S = an.data.summary || {};
  const audio = an.data.audio?.opus || null;
  const title = info?.title || an.data.set?.title || id;
  const unreliable = !!info?.unreliable;
  const recS = S.duration_s || info?.card.duration_s || m.durationMs / 1000;
  const nt = newTonight(m);

  const headlineWhy = () => why.why({
    title: 'Laughs in this set', who: 'measured', claim: `${fmt1(m.rate)} seconds of laughter per minute of material`,
    reason: `${heatWord(info?.roomHeat) || 'room heat unknown'} · over ${fmtDur(m.material_s)} of material.`,
    detail: (
      <View style={{ gap: 6 }}>
        <T v="small" testID="why-raw">Detector: {S.laughs ?? m.roomLaughs} laughs = {fmt1(S.laughs_per_min)} raw laughs/min. One long rolling laugh can count as several, so it isn’t the headline.</T>
        <T v="small">Laugh bars per laugh:</T>
        <ReceptionLegend kind="laugh" />
      </View>
    ),
    whoLine: `Measured automatically · not hand-checked yet${m.rateSource === 'derived' ? ' · room rate derived from the rows' : ''}.`,
  });

  const venueLine = (
    <Pressable testID="set-venue" accessibilityRole="button" accessibilityLabel="Show date and venue. Change" onPress={() => setEditVenue(true)} style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexWrap: 'wrap', minHeight: 32 }}>
      <T v="eb" testID="set-date">{fmtShortDate(info?.show.date)}</T>
      {info?.show.inferred && <AIBadge />}
      <T v="eb">· {info?.venue.name || '…'}</T>
      {info?.venue.guessed ? <AIBadge /> : info?.venue.yours ? <Yours /> : null}
    </Pressable>
  );

  const summary = (
    <Card testID="set-summary" style={{ gap: 12 }}>
      {unreliable ? (
        <View testID="audio-unreliable" style={{ gap: 6 }}>
          <T v="semi">⚠ Audio unreliable · not in trends</T>
          <T v="mut">We heard almost no laughs: {m.roomLaughs} in {fmtDur(m.material_s)}. That’s usually the recording, not you.</T>
        </View>
      ) : (
        <LaughHeadline testID="set-headline" value={m.rate} roomHeat={info?.roomHeat} onPress={headlineWhy}
          sub={info?.roomHeat != null ? null : undefined} />
      )}
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', rowGap: 10 }}>
        {[
          ['stat-pct', '% laughing', fmtPct(m.pct)],
          ['stat-rolling', 'rolling laughs/min', fmt1(m.rolling)],
          ['stat-longest', 'longest laugh', m.longest ? `${fmt1(m.longest.s)} s` : '–', m.longest?.title?.split(' (')[0]],
          ['stat-quiet', 'longest quiet', m.quiet ? fmtDur(m.quiet.s) : '–', m.quiet ? `at ${mmss(m.quiet.at_ms)}` : null],
        ].map(([tid, label, v, sub]) => (
          <View key={tid as string} testID={tid as string} style={{ width: wide ? '25%' : '50%', paddingRight: 8 }}>
            <T v="num" style={{ fontSize: 20 }}>{v}</T>
            <T v="small">{label}</T>
            {!!sub && <T v="small" numberOfLines={1} style={{ color: C.text2 }}>{sub}</T>}
          </View>
        ))}
      </View>
      <DensityStrip testID="timeline" durationMs={m.durationMs} laughs={an.data.laughs || []} quiet={m.quiet ? { at_ms: m.quiet.at_ms, dur_s: m.quiet.s } : null}
        segs={m.rows.map((r) => ({ idx: r.idx, start_ms: r.row.start_ms, end_ms: r.row.end_ms, lspm: r.lspm, nonBit: r.nonBit, word: r.shown.word }))}
        onPress={(idx) => jump(idx)} />
      <T v="small">{fmtDur(m.material_s)} of material in a {fmtDur(recS)} recording</T>
    </Card>
  );

  const wsBlock = align ? (
    <Card testID="vs-working-set" style={{ gap: 8 }}>
      <View style={s.row}><T v="eb" style={{ flex: 1 }}>vs Working set v{align.version}</T><AIBadge /></View>
      <T>{align.order.in_order.length} of {align.order.planned.length} in order{align.added.length ? ` · ${align.added.slice(0, 2).map((a) => a.title.split(' (')[0]).join(', ')} added` : ''}{align.dropped.length ? ` · ${align.dropped.length} dropped` : ''}</T>
      <List>
        {align.bits.map((b) => (
          <View key={b.joke_id} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 40 }}>
            <T v="num" style={{ width: 22, fontSize: 13, color: C.mut }}>{b.planned_pos}</T>
            <T style={{ flex: 1 }} numberOfLines={1}>{b.title.split(' (')[0]}</T>
            <T v="small">{b.performed ? `${fmtDur(b.time_s)} vs ${fmtDur(b.target_s)}` : 'dropped'}</T>
          </View>
        ))}
      </List>
    </Card>
  ) : null;

  const saveNew = (r: RowM) => act(async () => {
    const text = r.row.punchlines?.[r.row.punchlines.length - 1]?.text || (r.row.transcript || '').slice(0, 200);
    return [await post<any>('/api/ideas', { text, title: r.row.suggested_title || undefined, from: { set: id, perf: r.row.perf, start_ms: r.row.start_ms, end_ms: r.row.end_ms, note: 'saved from the Set screen' } })];
  }, `Saved as a joke${r.row.suggested_title ? `: ${r.row.suggested_title}` : ''}`);

  const newBlock = nt.length ? (
    <Card testID="new-tonight" style={{ gap: 10 }}>
      <T v="eb">New tonight</T>
      {nt.map((r) => {
        const line = r.row.punchlines?.[r.row.punchlines.length - 1]?.text || r.row.title;
        const refiled = r.row.status !== 'new';
        return (
          <View key={r.idx} testID={`new-${r.idx}`} style={{ gap: 6 }}>
            <T style={{ fontSize: 16 }}>“{line}”</T>
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
              <T v="small" style={{ flex: 1, minWidth: 140 }}>{fmt1(r.lspm)} laugh-s/min{r.comparable ? ` · ${fmtVs(r.vs)}` : ''}{refiled ? ` · still your best new line (you filed it as ${nonBitWord(r.row)})` : ''}</T>
              {audio && <Btn small v="ghost" label="▶" accessibilityLabel="Hear it" onPress={() => playSegment(audio, r.row.play?.start_ms ?? r.row.start_ms, r.row.play?.end_ms ?? r.row.end_ms, `new-${r.idx}`).catch((e) => ui.snack(e.message))} />}
              {r.row.saved_as ? <T v="small" style={{ color: C.amber }}>Saved as {r.row.saved_as.title || 'a joke'}</T> : <Btn testID={`save-${r.idx}`} small label="Save" onPress={() => saveNew(r)} />}
            </View>
          </View>
        );
      })}
    </Card>
  ) : null;

  const clipList = (clips.data?.clips?.length ? [...clips.data.clips].sort((a, b) => (a.rank ?? 99) - (b.rank ?? 99)).slice(0, 5).map((c) => ({ ...c, title: (c.title || '').split(' (')[0], why: c.score_breakdown?.laugh_s ? `${fmt1(c.score_breakdown.laugh_s.raw)} s of laughs` : '' })) : m.rows.filter((r) => !r.nonBit && r.comparable && r.vs != null).sort((a, b) => (b.vs! - a.vs!)).slice(0, 3)
    .map((r) => ({ idx: r.idx, joke_id: r.row.joke?.id || null, title: (r.row.joke?.title || r.row.title || '').split(' (')[0], start_ms: r.row.play?.start_ms ?? r.row.start_ms, end_ms: r.row.play?.end_ms ?? r.row.end_ms, vs_room: r.vs, why: `${fmtVs(r.vs)}` })));
  const clipsBlock = clipList.length && audio ? (
    <Card testID="best-clips" style={{ gap: 6 }}>
      <View style={s.row}><T v="eb" style={{ flex: 1 }}>Best clips</T>{!clips.data?.clips?.length && <T v="small">from the laughs</T>}</View>
      {clipList.map((c, i) => (
        <Pressable key={i} testID={`clip-${i}`} accessibilityRole="button" accessibilityLabel={`Play ${c.title}`} onPress={() => playSegment(audio, c.start_ms, c.end_ms, `clip-${i}`).catch((e) => ui.snack(e.message))}
          style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48 }}>
          <View style={{ width: 36, height: 36, borderRadius: 18, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.bg }}>▶</Text></View>
          <View style={{ flex: 1 }}><T numberOfLines={1}>{c.title}</T><T v="small">{mmss(c.start_ms)} · {fmtSec((c.end_ms - c.start_ms) / 1000)}{c.why ? ` · ${c.why}` : ''}</T></View>
        </Pressable>
      ))}
    </Card>
  ) : null;

  const rows = (
    <View testID="rows" style={{ gap: 0 }}>
      <T v="eb" style={{ marginBottom: 4 }}>Bit by bit</T>
      {m.rows.map((r) => (
        <SetRowView key={r.idx} r={r} open={!!open[r.idx]} audio={audio} setId={id}
          onToggle={() => setOpen((o) => ({ ...o, [r.idx]: !o[r.idx] }))} onThisWas={() => setThisWas(r)} onRate={() => setRateRow(r)} />
      ))}
    </View>
  );

  const footer = (
    <View style={{ gap: 10 }}>
      <T v="small">Laughs are measured automatically and haven’t been hand-checked yet.</T>
      <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
        {!unreliable && <Btn testID="mark-unreliable" small v="ghost" label="Mark audio as bad" onPress={async () => {
          try { const u = await editSet(id, { audio_unreliable: true }); ui.snack('Marked: audio unreliable · left out of trends', u); }
          catch (e: any) { ui.snack(`The server can’t store this yet (${e.message})`); }
        }} />}
        {unreliable && info?.unreliableYours && <Btn small v="ghost" label="Audio is fine" onPress={async () => { try { const u = await editSet(id, { audio_unreliable: false }); ui.snack('Back in trends', u); } catch (e: any) { ui.snack(`The server can’t store this yet (${e.message})`); } }} />}
      </View>
    </View>
  );

  return (
    <Screen title="Set" back testID="set" scrollRef={scroll}>
      <FixtureBanner show={sets.fixture} what="GET /api/venues (per-set laugh stats)" />
      <View style={{ gap: 2 }}>
        {venueLine}
        <T v="h1" testID="set-title">{title}</T>
      </View>
      {wide ? (
        <View style={{ flexDirection: 'row', gap: 20, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, gap: 16, minWidth: 0 }}>{summary}{rows}</View>
          <View style={{ flex: 2, gap: 16, minWidth: 0 }}>{newBlock}{clipsBlock}{wsBlock}{footer}</View>
        </View>
      ) : (
        <>{summary}{wsBlock}{newBlock}{clipsBlock}{rows}{footer}</>
      )}

      <ThisWasSheet r={thisWas} onClose={() => setThisWas(null)} act={act} />
      <RateSheet r={rateRow} onClose={() => setRateRow(null)} act={act} />
      <VenueSheet visible={editVenue} onClose={() => setEditVenue(false)} id={id} info={info} />
    </Screen>
  );
}

// ------------------------------------------------------------------ a row
const SetRowView = memo(function SetRowView({ r, open, audio, setId, onToggle, onThisWas, onRate }: {
  r: RowM; open: boolean; audio: string | null; setId: string; onToggle: () => void; onThisWas: () => void; onRate: () => void;
}) {
  const ui = useUI();
  const why = useWhy();
  const boards = useBoards();
  const row = r.row;
  const play = () => audio && playSegment(audio, row.play?.start_ms ?? Math.max(0, row.start_ms - 4000), row.play?.end_ms ?? row.end_ms, `row-${r.idx}`).catch((e) => ui.snack(e.message));
  const yoursKind = row.kind_by === 'you';
  if (r.nonBit) {
    const first = (row.transcript || '').split(/\s+/).slice(0, 7).join(' ');
    return (
      <View testID={`row-${r.idx}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48, borderTopWidth: 1, borderTopColor: C.border }}>
        <T v="num" style={{ width: 44, fontSize: 13, color: C.dim }}>{mmss(row.start_ms)}</T>
        <Pressable testID={`title-${r.idx}`} accessibilityRole="button" accessibilityLabel={`${nonBitWord(row)}. Change what this was`} onPress={onThisWas} style={{ flex: 1, minHeight: 44, justifyContent: 'center' }}>
          <Text numberOfLines={1} style={{ fontFamily: F.body, fontSize: 14, color: C.dim }}>{nonBitWord(row)}{first ? ` · “${first}…”` : ''}{yoursKind ? ' · yours' : ''}</Text>
        </Pressable>
        {audio && <PlayBtn testID={`hear-${r.idx}`} onPress={play} />}
      </View>
    );
  }
  const t = row.status === 'new' ? 'New material' : (row.joke?.title || row.title || '').split(' (')[0];
  const matchAi = row.match?.state === 'ai' || row.kind_by === 'ai';
  const ratingYours = r.shown.by === 'you';
  return (
    <View testID={`row-${r.idx}`} style={{ borderTopWidth: 1, borderTopColor: C.border, paddingVertical: 8, gap: 6 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <T v="num" style={{ width: 44, fontSize: 13, color: C.mut }}>{mmss(row.start_ms)}</T>
        <Pressable testID={`title-${r.idx}`} accessibilityRole="button" accessibilityLabel={`${t}. This was…`} onPress={onThisWas} style={{ flex: 1, minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ flexShrink: 1, fontFamily: F.semi, fontSize: 16, color: row.status === 'new' ? C.amber : C.text }}>{t}</Text>
          {matchAi && row.status !== 'new' ? <AIBadge testID={`ai-${r.idx}`} /> : row.match?.state && row.match.state !== 'ai' ? <Yours /> : null}
        </Pressable>
        {audio && <PlayBtn testID={`hear-${r.idx}`} onPress={play} />}
      </View>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingLeft: 52, minHeight: 36 }}>
        <Pressable testID={`metrics-${r.idx}`} onPress={onToggle} accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={`${fmt1(r.lspm)} laugh-seconds per minute. ${open ? 'Less' : 'More'}`}
          style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 36 }}>
          <T v="num" style={{ width: 34, fontSize: 15 }}>{fmt1(r.lspm)}</T>
          <VsRoomBar vs={r.vs} comparable={r.comparable} testID={`vs-${r.idx}`} />
        </Pressable>
        <Pressable testID={`rate-${r.idx}`} accessibilityRole="button" accessibilityLabel={`Rating ${r.shown.word || 'none'}. Change`} onPress={onRate} hitSlop={8} style={{ minHeight: 36, minWidth: 44, alignItems: 'flex-end', justifyContent: 'center' }}>
          {r.shown.word ? <RatingTag word={r.shown.word} yours={ratingYours} testID={`rating-${r.idx}`} /> : <Text style={{ color: C.dim, fontSize: 12 }}>rate</Text>}
        </Pressable>
      </View>
      {open && (
        <View testID={`detail-${r.idx}`} style={{ paddingLeft: 52, gap: 8 }}>
          {(row.punchlines || []).map((pl: any, i: number) => (
            <Pressable key={i} testID={`punch-${r.idx}-${i}`} accessibilityRole="button" accessibilityLabel={`Hear: ${pl.text}`} onPress={() => audio && playSegment(audio, Math.max(0, pl.at_ms - 4000), pl.at_ms + 6000, `pl-${r.idx}-${i}`).catch((e) => ui.snack(e.message))}
              style={[s.punch, { gap: 4, paddingVertical: 4 }]}>
              <T style={{ fontSize: 15 }}>{pl.text}</T>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}><ReceptionMeter bars={pl.bars} size={12} word={BAR_WORDS[pl.bars] || ''} /><T v="small">▶ {mmss(pl.at_ms)}</T></View>
            </Pressable>
          ))}
          {!!row.rating_basis && <WhyLine testID={`basis-${r.idx}`} text={row.rating_basis.replace(/\d+\.\d{2,}/g, (x: string) => fmt1(Number(x)))} onPress={() => why.why({
            title: 'How it did', who: 'measured', claim: `${fmt1(r.lspm)} laugh-s/min${r.comparable ? ` · ${fmtVs(r.vs)}` : ''}`, reason: row.rating_basis,
            detail: <ReceptionLegend kind="room" />, whoLine: 'Measured from the tape. The word comes from × room: killed ≥ 1.5×, worked 0.8–1.5×, meh 0.2–0.8×, died < 0.2×.',
          })} />}
          <T v="small">{r.ttfl != null ? `first laugh ${Math.round(r.ttfl)} s in` : 'no laugh in this span'}</T>
          {row.joke && (
            <WhyLine testID={`match-${r.idx}`} text={`Matched: ${row.joke.title.split(' (')[0]}${row.version?.doc_title ? ` · ${row.version.doc_title} ${row.version.date || ''}` : ''}`} onPress={onThisWas} />
          )}
          {row.script_diff && (
            <View testID={`diff-${r.idx}`} style={{ gap: 4 }}>
              <T v="small" style={{ color: C.text2 }}>{row.script_diff.on_script_pct}% on script</T>
              <Diff lines={row.script_diff.dropped || []} kind="dropped" />
              <Diff lines={row.script_diff.added || []} kind="added" />
            </View>
          )}
          {(row.similar || []).slice(0, 2).map((x: any, i: number) => <WhyLine key={`sim${i}`} testID={`sim-${r.idx}-${i}`} text={`Same shape: ${x.title.split(' (')[0]} · ${x.why}`} onPress={() => router.push(`/joke/${encodeURIComponent(x.joke_id)}` as any)} />)}
          {(row.hints || []).slice(0, 2).map((h: any, i: number) => <WhyLine key={`h${i}`} testID={`hint-${r.idx}-${i}`} text={`${h.name}: ${h.why}`} />)}
          <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
            {row.joke && <Btn small v="ghost" label="Open bit" onPress={() => router.push(`/joke/${encodeURIComponent(row.joke.id)}` as any)} />}
            {row.joke && <AddBtn testID={`row-add-${r.idx}`} onPress={() => boards.add(row.joke.id, row.joke.title.split(' (')[0])} onLongPress={() => boards.pick(row.joke.id, row.joke.title)} />}
          </View>
        </View>
      )}
    </View>
  );
});

function Diff({ lines, kind }: { lines: string[]; kind: 'dropped' | 'added' }) {
  const [all, setAll] = useState(false);
  if (!lines.length) return null;
  const shown = all ? lines : lines.slice(0, 3);
  return (
    <View style={{ gap: 3 }}>
      {shown.map((l, i) => (
        <Text key={i} numberOfLines={all ? undefined : 2} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: kind === 'dropped' ? C.redSoft : C.blue }}>
          {kind === 'dropped' ? '− ' : '+ '}{l}
        </Text>
      ))}
      {lines.length > 3 && <TextBtn label={all ? 'Fewer' : `Show all ${lines.length} ${kind}`} color={C.mut} onPress={() => setAll((x) => !x)} />}
    </View>
  );
}

const PlayBtn = ({ onPress, testID }: { onPress: () => void; testID?: string }) => (
  <Pressable testID={testID} accessibilityRole="button" accessibilityLabel="Hear it" onPress={onPress}
    style={{ width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' }}>
    <View style={{ width: 32, height: 32, borderRadius: 16, backgroundColor: C.amberSoft, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.amber, fontSize: 13 }}>▶</Text></View>
  </Pressable>
);

// ------------------------------------------------------------------ sheets
type Act = (fn: () => Promise<{ indexes: number[] }[]>, msg: string) => Promise<void>;
function ThisWasSheet({ r, onClose, act }: { r: RowM | null; onClose: () => void; act: Act }) {
  const [q, setQ] = useState('');
  const res = useApi<{ items: any[] }>(q.trim().length >= 2 ? `/api/jokes?q=${encodeURIComponent(q.trim())}&limit=6` : null, { cache: false });
  useEffect(() => setQ(''), [r]);
  if (!r) return <Sheet visible={false} onClose={onClose} label="This was…">{null}</Sheet>;
  const row = r.row;
  const perf = row.perf;
  const cur = row.joke?.title?.split(' (')[0];
  const opt = (testID: string, label: string, sub: string | null, fn: () => Promise<{ indexes: number[] }[]>, msg: string, on?: boolean) => (
    <Pressable key={testID} testID={testID} accessibilityRole="button" onPress={() => { onClose(); act(fn, msg); }}
      style={[s.inner, { minHeight: 52, justifyContent: 'center', gap: 2 }, on && { borderColor: C.amber }]}>
      <T v="semi">{label}</T>{!!sub && <T v="small">{sub}</T>}
    </Pressable>
  );
  return (
    <Sheet visible onClose={onClose} label="This was…" testID="this-was-sheet">
      <T v="eb">{mmss(row.start_ms)} · {row.match?.state === 'ai' ? 'the AI read' : 'yours'}</T>
      <T v="h2" style={{ marginBottom: 4 }}>This was…</T>
      {!!row.match_reason && <T v="mut" style={{ marginBottom: 8 }}>{row.match_reason}</T>}
      <View style={{ gap: 8 }}>
        {row.joke && opt('was-current', cur!, `the matched bit${row.match_confidence ? ` · ${Math.round(row.match_confidence * 100)}% sure` : ''} · This is right`, async () => [await setRow({ perf, op: 'confirm' })], `${cur}: yours now`, true)}
        <View style={[s.inner, { gap: 6 }]}>
          <T v="semi">Another bit</T>
          <TextInput testID="match-search" value={q} onChangeText={setQ} placeholder="Search your bits" placeholderTextColor={C.dim}
            style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          {(res.data?.items || []).map((j) => (
            <ListRow key={j.id} testID={`match-pick-${j.id}`} title={j.title} onPress={() => { onClose(); act(async () => [await setRow({ perf, op: 'match', joke: j.id, reason: 'changed on the set page' })], `Now: ${j.title.split(' (')[0]}`); }} />
          ))}
        </View>
        {row.status !== 'new' && opt('was-new', 'New material', 'Not a bit you have yet', async () => [await setRow({ perf, op: 'new' })], 'Marked as new material')}
        {row.status !== 'crowd_work' && opt('was-crowd', 'Crowd work', 'No rating, no metrics', async () => [await setRow({ perf, op: 'crowd' })], 'Marked as crowd work')}
        {row.status !== 'banter' && opt('was-banter', 'Banter', 'Walk-on, sign-off, chat', async () => [await decide({ target: 'node', action: 'edit', id: perf, after: { kind: 'banter' } })], 'Marked as banter')}
        {(row.status === 'crowd_work' || row.status === 'banter') && opt('was-bit', 'A bit after all', null, async () => [await setRow({ perf, op: 'bit' })], 'Back to a bit')}
      </View>
      {row.ai_status && row.ai_status !== row.status && <T v="small" style={{ marginTop: 10 }}>The AI first read it as {row.ai_status.replace('_', ' ')}.</T>}
    </Sheet>
  );
}

function RateSheet({ r, onClose, act }: { r: RowM | null; onClose: () => void; act: Act }) {
  if (!r) return <Sheet visible={false} onClose={onClose} label="Rating">{null}</Sheet>;
  const derived = r.comparable ? ratingFromVsRoom(r.vs, r.row.laughs?.laugh_s, r.row.laughs?.window_s) : null;
  return (
    <Sheet visible onClose={onClose} label="Rating" testID="rate-sheet">
      <T v="eb">{(r.row.joke?.title || r.row.title || '').split(' (')[0]}</T>
      <T v="h2" style={{ marginBottom: 4 }}>{fmt1(r.lspm)} laugh-s/min{r.comparable ? ` · ${fmtVs(r.vs)}` : ''}</T>
      <T v="mut" style={{ marginBottom: 10 }}>{r.comparable ? `The numbers say “${derived}”.` : 'The room was too quiet to compare.'} Your word wins.</T>
      <View style={{ gap: 8 }}>
        {RATINGS.map((w: Rating) => (
          <Btn key={w} testID={`rate-${r.idx}-${w}`} label={`${w}${w === derived ? ' · the numbers' : ''}`} v={w === r.shown.word ? 'primary' : 'plain'}
            onPress={() => { onClose(); act(async () => [await setRow({ perf: r.row.perf, op: 'rate', rating: w })], `Rated ${w}`); }} />
        ))}
      </View>
    </Sheet>
  );
}

function VenueSheet({ visible, onClose, id, info }: { visible: boolean; onClose: () => void; id: string; info: any }) {
  const ui = useUI();
  const [venue, setVenue] = useState('');
  const [date, setDate] = useState('');
  useEffect(() => {
    if (visible) { setVenue(info?.venue.name?.replace(/\?$/, '') || ''); setDate(info?.show.date || ''); }
  }, [visible]); // eslint-disable-line react-hooks/exhaustive-deps
  const input = { minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 } as const;
  return (
    <Sheet visible={visible} onClose={onClose} label="Show date and venue" testID="venue-sheet">
      <T v="h2">Show date and venue</T>
      <T v="mut" style={{ marginVertical: 6 }}>
        {info?.show.source === 'title' ? 'The date was read from the video title.' : info?.show.source === 'description' ? 'The date was read from the video description.' : info?.show.source === 'yours' ? 'You set the date.' : 'The date is the upload date.'} The venue: {info?.venue.raw}.
      </T>
      <View style={{ gap: 8 }}>
        <T v="small">Date (YYYY-MM-DD)</T>
        <TextInput testID="edit-date" value={date} onChangeText={setDate} style={input} />
        <Btn testID="save-date" small label="Save date" disabled={!/^\d{4}-\d\d-\d\d$/.test(date) || date === info?.show.date} onPress={async () => {
          onClose();
          try { const u = await editSet(id, { show_date: date }); ui.snack(`Show date: ${fmtShortDate(date)}`, u); }
          catch (e: any) { ui.snack(`The server can’t store this yet (${e.message})`); }
        }} />
        <T v="small">Venue</T>
        <TextInput testID="edit-venue" value={venue} onChangeText={setVenue} style={input} />
        <Btn testID="save-venue" small label="Save venue" disabled={!venue.trim()} onPress={async () => {
          onClose();
          try { const u = await editSet(id, { venue: venue.trim() }); ui.snack(`Venue: ${venue.trim()}`, u); }
          catch (e: any) { ui.snack(`The server can’t store this yet (${e.message})`); }
        }} />
      </View>
    </Sheet>
  );
}
