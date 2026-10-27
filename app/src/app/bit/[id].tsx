// The bit, in detail (one tap down from the joke map, SIMPLE.md): stage, how it does on stage, versions, brainstorm,
// notes. Was the joke workspace (UX_SPEC 4.3) at /joke/<id>; /joke/<id> is now the joke map.
// Three parallel requests, no waterfall: the joke (/api/jokes/<id>), its connections (/api/jokes/<id>/connections)
// and the sets index (show dates, venues, room rates). The Working set view is already cached app-wide.
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { EdgeView, decide, get, post, undo, useApi } from '@/lib/api';
import { ConnGroup, ConnGroupKey, ConnItem, Connections, PunchUp, WSView, creative, useCreative } from '@/lib/creative';
import { confWord, fmt1, fmtDur, fmtShortDate, fmtVs, mergeNights, trendWord } from '@/lib/laughs';
import { Sparkline } from '@/lib/metrics';
import { useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { playSegment, youtubeSource } from '@/lib/media';
import { C, F, STAGES, stageInfo, stageWord } from '@/lib/theme';
import {
  AIBadge, AddBtn, Btn, Card, Chips, Chip, Empty, ErrorCard, FixtureBanner, IconBtn, Icon, List, ListRow, Proven, Screen, Seg, Sheet, T, TextBtn, Yours, s, useUI, useWide,
} from '@/lib/ui';
import { WhyLine, useWhy } from '@/lib/why';

type Tab = 'connect' | 'brainstorm' | 'versions' | 'notes';
const GROUP_ORDER: ConnGroupKey[] = ['next', 'tags', 'same_world', 'callbacks', 'same_shape', 'contrasts'];
const GROUP_TITLE: Record<ConnGroupKey, string> = {
  next: 'What could come next', tags: 'Lines that could tag it', same_world: 'Same world', callbacks: 'Callbacks', same_shape: 'Same shape', contrasts: 'Contrast / reset',
};
const KIND_FOR: { key: string; label: string; kind: string }[] = [
  { key: 'next', label: 'Comes next', kind: 'segue' }, { key: 'tags', label: 'Tags it', kind: 'tag_of' }, { key: 'world', label: 'Same world', kind: 'same_premise' },
  { key: 'callback', label: 'Calls back to', kind: 'callback_to' }, { key: 'shape', label: 'Same shape', kind: 'same_shape' }, { key: 'contrast', label: 'Contrast', kind: 'custom' },
];

const proven = (it: ConnItem) => (it.basis || []).includes('back_to_back');
/** proven > yours > inferred > proposed, then score. */
const rank = (it: ConnItem) => (proven(it) ? 0 : it.source === 'his' ? 1 : it.source === 'inferred' ? 2 : 3);
export function rankGroup(items: ConnItem[]) {
  return [...items].sort((a, b) => rank(a) - rank(b) || b.score - a.score || a.title.localeCompare(b.title));
}
/** Below 0.5 and not proven/yours: a long shot (one collapsed list at the bottom of the tab). */
const longShot = (it: ConnItem) => it.score < 0.5 && !proven(it) && it.source !== 'his';

/** Cut the span down to the sentences around the reveal and mark the reveal. */
function heroParts(text: string, reveal: string | null) {
  const t = text.replace(/\s+/g, ' ').trim();
  let at = reveal ? t.toLowerCase().indexOf(reveal.toLowerCase()) : -1;
  if (at < 0 && reveal) {
    const w = reveal.toLowerCase().split(' ').slice(-3).join(' ');
    at = t.toLowerCase().indexOf(w);
    if (at >= 0) reveal = t.slice(at, at + w.length);
  }
  if (at < 0 || !reveal) {
    const short = t.length > 260 ? `…${t.slice(-240)}` : t;
    return { before: short, reveal: '', after: '' };
  }
  // about two sentences of lead-in (≤ 180 chars, cut at a word) and the rest of the reveal's sentence (≤ 110)
  const leadStart = Math.max(0, at - 180);
  let st = t.lastIndexOf('. ', at - 1);
  st = st >= leadStart ? st + 2 : t.indexOf(' ', leadStart) + 1;
  if (at <= 180) st = 0;
  const endS = t.indexOf('. ', at + reveal.length);
  const tailEnd = endS >= 0 && endS - (at + reveal.length) <= 110 ? endS + 1 : Math.min(t.length, t.indexOf(' ', at + reveal.length + 100) > 0 ? t.indexOf(' ', at + reveal.length + 100) : t.length);
  return {
    before: `${st > 0 ? '…' : ''}${t.slice(st, at)}`,
    reveal: t.slice(at, at + reveal.length),
    after: `${t.slice(at + reveal.length, tailEnd)}${tailEnd < t.length ? ' …' : ''}`,
  };
}

export default function Joke() {
  const { id: rawId, tab: tab0, unit } = useLocalSearchParams<{ id: string; tab?: string; unit?: string }>();
  const id = decodeURIComponent(String(rawId || ''));
  const ui = useUI();
  const why = useWhy();
  const boards = useBoards();
  const wide = useWide();
  const d = useApi<any>(`/api/jokes/${encodeURIComponent(id)}`);
  const conn = useCreative<Connections>(`conn:${id}`, () => creative.connections(id, 10));
  const sets = useSets();
  const wsId = ui.workingSet?.id || null;
  const ws = useCreative<WSView>(wsId ? `ws:${wsId}` : null, () => creative.workingset(wsId!));
  const [tab, setTab] = useState<Tab>((tab0 as Tab) || 'connect');
  const [stageSheet, setStageSheet] = useState(false);
  const [more, setMore] = useState(false);
  const [full, setFull] = useState(false);
  useEffect(() => {
    if (tab0) setTab(tab0 as Tab);
  }, [tab0]);

  const J = d.data?.joke;
  const title = J ? String(J.title).split(' (')[0] : '';
  const inWs = !!ws.data?.version.bits.some((b) => b.joke_id === id);
  const wsBit = ws.data?.bits.find((b) => b.joke_id === id);

  // ---- nights: one per set, fragments merged; unreliable rooms excluded from avg and trend
  const nights = useMemo(() => {
    const perfs: any[] = (d.data?.performances || []).filter((p: any) => p.edge?.bucket !== 'rejected');
    // only sets the API lists as his shows (v1 also holds perfs in sets it hides)
    const raw = perfs.filter((p) => sets.index.has(typeof p.set === 'string' ? p.set : p.set?.id || '')).map((p) => {
      const sid: string = typeof p.set === 'string' ? p.set : p.set?.id || '';
      const info = sets.index.get(sid);
      // the summary on the perf when the backend has it; else the laugh events inside the span
      const L = p.perf?.laughs || {
        laugh_s: (p.laughs || []).reduce((a: number, l: any) => a + Math.max(0, (Math.min(l.end_ms, p.perf?.end_ms ?? l.end_ms) - Math.max(l.start_ms, p.perf?.start_ms ?? l.start_ms)) / 1000), 0),
        window_s: p.perf?.end_ms != null && p.perf?.start_ms != null ? (p.perf.end_ms - p.perf.start_ms) / 1000 : 0,
      };
      const first = (p.laughs || []).map((l: any) => l.start_ms).sort((a: number, b: number) => a - b)[0];
      return {
        set: sid, date: info?.show.date || sid.slice(4, 14), label: info ? `${info.venue.name.replace(/\?$/, '')} '${(info.show.date || '').slice(2, 4)}` : sid.slice(4, 8),
        laugh_s: L.laugh_s || 0, window_s: L.window_s || 0, excluded: !!info?.unreliable, rate: info?.lspm ?? null,
        ttfl: first != null && p.perf?.start_ms != null ? (first - p.perf.start_ms) / 1000 : null, perf: p,
      };
    });
    return mergeNights(raw).map((n: any) => ({ ...n, vs: n.rate ? n.laugh_s_per_min / n.rate : null })).sort((a, b) => String(a.date).localeCompare(String(b.date)));
  }, [d.data, sets.index]);
  const counted = nights.filter((n) => !n.excluded);
  const best = counted.length ? counted.reduce((a, b) => (b.laugh_s_per_min > a.laugh_s_per_min ? b : a)) : null;
  const avg = counted.length ? counted.reduce((a, n) => a + n.laugh_s_per_min, 0) / counted.length : null;
  const ttfls = counted.map((n) => n.ttfl).filter((x): x is number => x != null).sort((a, b) => a - b);
  const medTtfl = ttfls.length ? ttfls[Math.floor(ttfls.length / 2)] : null;
  const trend = trendWord(nights.map((n) => ({ v: n.laugh_s_per_min, excluded: n.excluded })));

  // ---- hero: the best stage version's verbatim span, else the newest setup+punch unit
  const hero = useMemo(() => {
    const reveal = J?.anatomy?.reveal || null;
    if (best) {
      const p = best.perf;
      const text = p.perf?.text || p.punch?.text || '';
      const info = sets.index.get(best.set);
      return { kind: 'stage' as const, text, reveal: reveal && text.toLowerCase().includes(reveal.toLowerCase()) ? reveal : (p.punch?.text || reveal),
        caption: `On stage · ${info?.venue.name.replace(/\?$/, '') || ''} · ${fmtShortDate(best.date)}`, play: p.play, audio: p.audio || youtubeSource(p.reference), set: best.set };
    }
    const units: any[] = (d.data?.versions || []).flatMap((v: any) => v.units);
    const u = [...units].reverse().find((x) => x.kind === 'setup+punch') || units[units.length - 1];
    if (!u) return null;
    return { kind: 'doc' as const, text: u.text || '', reveal: u.anatomy?.reveal || reveal, caption: `Written · ${u.doc_title || 'doc'} · ${fmtShortDate(u.date)}`, unit: u.id };
  }, [J, best, d.data, sets.index]);
  const hp = hero ? heroParts(hero.text, hero.reveal) : null;

  const craft = useMemo(() => {
    const seen = new Set<string>();
    return (d.data?.labels || []).filter((l: EdgeView) => l.state !== 'rejected' && (l.other?.kind === 'structure' || l.other?.kind === 'device'))
      .map((l: EdgeView) => l.other!.title).filter((n: string) => (seen.has(n) ? false : (seen.add(n), true))).slice(0, 4);
  }, [d.data]);

  const writtenGroups = (d.data?.versions || []).length;
  const rooms = new Set(nights.map((n) => n.set)).size;
  const span = J ? `${(J.first_seen || '').slice(0, 4)} → ${(J.last_seen || '').slice(0, 4)}` : '';
  const ai = d.data?.ai || {};
  const stageAi = !!ai.stage_ai && !ai.stage_confirmed;

  const stageWhy = () => why.why({
    title: 'Stage', who: stageAi ? 'ai' : 'yours', claim: `${title}: ${stageWord(J?.stage)}`, reason: ai.stage_reason_ai || J?.stage_reason || null, confidence: ai.stage_confidence ?? null,
    evidence: nights.slice(-3).map((n) => ({ node: n.perf.perf.id, quote: n.perf.punch?.text || '', start_ms: n.perf.punch?.start_ms ?? null, label: `${n.label} · ${fmt1(n.laugh_s_per_min)} laugh-s/min` })).filter((e) => e.quote),
    change: { label: 'Change stage…', run: () => setStageSheet(true) },
    right: stageAi ? async () => {
      const w = await decide({ target: 'node', action: 'confirm', id, after: { status: 'confirmed' } });
      ui.snack(`${stageWord(J?.stage)} is right`, async () => { await undo(w.indexes); d.reload(); });
      d.reload();
    } : undefined,
    testID: 'stage-why',
  });

  const tryTonight = async () => {
    try {
      const t = await get<any>('/api/today');
      const items = (t.tonight?.items || []).map((x: any) => ({ joke: x.id, done: !!x.done }));
      if (items.some((x: any) => x.joke === id)) return ui.snack(`${title} is already on tonight’s list`);
      const w = await post<any>('/api/tonight', { items: [...items, { joke: id, done: false }] });
      ui.snack(`${title} · try tonight`, async () => { await undo(w.indexes); });
    } catch (e: any) {
      ui.snack(`Couldn't add: ${e.message}`);
    }
  };
  const lockBit = async () => {
    if (!wsId) return;
    const locked = !!wsBit?.locked;
    try {
      await creative.lock(wsId, { joke_id: id, unlock: locked });
      ws.reload();
      ui.snack(locked ? `${title} unlocked` : `${title} locked: the AI stops drafting changes to it`, async () => { await creative.lock(wsId, { joke_id: id, unlock: !locked }); ws.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't lock: ${e.message}`);
    }
  };

  const header = (
    <View style={{ gap: 6 }}>
      <Pressable testID="stage-line" accessibilityRole="button" accessibilityLabel={`Stage ${stageWord(J?.stage)}${stageAi ? ', set by the AI' : ''}. Why?`} onPress={stageWhy}
        style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexWrap: 'wrap', minHeight: 32 }}>
        <Text testID="joke-stage" style={{ fontFamily: F.g7, fontSize: 12, letterSpacing: 0.9, color: stageInfo(J?.stage).fg }}>{stageWord(J?.stage).toUpperCase()}</Text>
        {stageAi ? <AIBadge word={confWord(ai.stage_confidence)} testID="ai-stage" /> : <Yours />}
        <T v="small">· {span} · {writtenGroups + nights.length} versions · {rooms} {rooms === 1 ? 'room' : 'rooms'}</T>
      </Pressable>
      <T v="h1" testID="joke-title">{title}</T>
      {!!J?.summary && <T v="small" numberOfLines={full ? undefined : 1} style={{ fontStyle: 'italic' }} testID="joke-caption">AI summary: {J.summary}</T>}
    </View>
  );

  const heroCard = hero && hp ? (
    <Card testID="joke-hero" style={{ gap: 10 }}>
      <Text maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }} testID="joke-text">
        {full ? hero.text.replace(/\s+/g, ' ') : <>{hp.before}{!!hp.reveal && <Text testID="hero-reveal" style={s.rv}>{hp.reveal}</Text>}{hp.after}</>}
      </Text>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <T v="small" testID="hero-caption" style={{ flex: 1, minWidth: 160 }}>{hero.caption}</T>
        {hero.kind === 'stage' && hero.audio && (
          <Btn testID="hero-play" small label={`▶ ${fmtDur((hero.play?.start_ms || 0) / 1000)}`} onPress={() => playSegment(hero.audio, hero.play.start_ms, hero.play.end_ms, `hero-${id}`).catch((e) => ui.snack(e.message))} />
        )}
        <TextBtn testID="hero-full" label={full ? 'Less' : 'Full text'} color={C.mut} onPress={() => setFull((f) => !f)} />
      </View>
      {!!craft.length && <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>{craft.map((c: string) => <View key={c} style={{ minHeight: 28, paddingHorizontal: 10, borderWidth: 1, borderColor: C.border, borderRadius: 8, justifyContent: 'center' }}><Text style={{ fontFamily: F.body, fontSize: 13, color: C.text2 }}>{c}</Text></View>)}</View>}
    </Card>
  ) : null;

  const onStage = nights.length ? (
    <Card testID="onstage" style={{ gap: 8 }}>
      <View style={s.row}>
        <T v="eb" style={{ flex: 1 }}>On stage</T>
        <T v="small" testID="trend-word">{trend}</T>
      </View>
      <Sparkline testID="sparkline" nights={nights.map((n) => ({ key: n.set, label: n.label, value: n.laugh_s_per_min, excluded: n.excluded, best: best?.set === n.set }))}
        onPress={(k) => {
          const n = nights.find((x) => x.set === k);
          router.push(`/set/${encodeURIComponent(k)}?at=${n?.perf.perf.start_ms ?? ''}&play=1` as any);
        }} />
      <T testID="onstage-stats" style={{ fontSize: 15 }}>
        avg {fmt1(avg)} laugh-s/min · {counted.length} {counted.length === 1 ? 'night' : 'nights'}{best ? ` · best ${fmt1(best.laugh_s_per_min)} (${best.label})` : ''}
      </T>
      <T v="small">{medTtfl != null ? `1st laugh ~${Math.round(medTtfl)} s in` : ''}{nights.some((n) => n.excluded) ? `${medTtfl != null ? ' · ' : ''}${nights.filter((n) => n.excluded).map((n) => n.label).join(', ')} greyed: audio unreliable` : ''}</T>
    </Card>
  ) : J ? (
    <Card testID="onstage-empty" style={{ gap: 8 }}>
      <T v="eb">On stage</T>
      <T v="mut">Not on tape yet.</T>
      <Btn small label="Add to Try tonight ＋" onPress={tryTonight} />
    </Card>
  ) : null;

  const footer = J ? (
    <WhyLine testID="stage-history" text={`${ai.from_stage ? `from ${stageWord(ai.from_stage).toLowerCase()}, ` : ''}${stageAi ? 'moved by the AI' : 'set by you'}${ai.stage_reason_ai ? `: hit in ${d.data?.rooms?.ai_hit_sets ?? rooms} rooms` : ''}`} onPress={stageWhy} />
  ) : null;

  const actions = J ? (
    <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
      <Btn testID="joke-add-board" v="add" small label="＋ Board" onPress={() => boards.add(id, title)} onLongPress={() => boards.pick(id, title)} />
      {inWs && <Btn testID="joke-try" small label="Try tonight" onPress={tryTonight} />}
      {inWs && <Btn testID="joke-lock" small label={wsBit?.locked ? 'Unlock' : 'Lock'} icon={<Icon name={wsBit?.locked ? 'unlock' : 'lock'} size={16} />} onPress={lockBit} />}
    </View>
  ) : null;

  const tabs = <Seg testID="joke-tabs" items={wide ? [['brainstorm', 'Brainstorm'], ['notes', 'Notes']] : [['connect', 'Connect'], ['brainstorm', 'Brainstorm'], ['versions', 'Versions'], ['notes', 'Notes']]} value={wide && (tab === 'connect' || tab === 'versions') ? 'brainstorm' : tab} onChange={setTab} />;
  const connect = <ConnectTab id={id} title={title} conn={conn.data} loading={conn.loading} error={conn.error} wide={wide} labels={d.data?.labels || []} onReload={conn.reload} />;
  const versions = <VersionsTab d={d.data} nights={nights} bestSet={best?.set || null} unit={unit ? decodeURIComponent(String(unit)) : null} onChanged={d.reload} />;
  const brainstorm = <BrainstormTab id={id} title={title} wsId={inWs ? wsId : null} locked={!!wsBit?.locked} conn={conn.data} />;
  const notes = <NotesTab J={J} conn={conn.data} />;

  const right = (
    <View style={{ flexDirection: 'row' }}>
      <IconBtn testID="joke-more" label="More" onPress={() => setMore(true)}><Icon name="more" /></IconBtn>
    </View>
  );

  return (
    <Screen title="Joke" back right={right} testID="joke">
      <FixtureBanner show={conn.fixture} what="GET /api/jokes/{id}/connections" />
      {d.error && !d.data ? <ErrorCard error={d.error} onRetry={d.reload} /> : null}
      {!J && !d.error && <Empty text="Loading the bit…" />}
      {J && (wide ? (
        <View style={{ flexDirection: 'row', gap: 24, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, gap: 16, minWidth: 0 }}>
            {header}{actions}{heroCard}{onStage}
            <T v="eb">Versions</T>{versions}{footer}
          </View>
          <View style={{ flex: 2, gap: 12, minWidth: 0 }} testID="joke-rail">
            <T v="eb">Connect</T>{connect}
            {tabs}{tab === 'notes' ? notes : brainstorm}
          </View>
        </View>
      ) : (
        <>
          {header}{actions}{heroCard}{onStage}{tabs}
          {tab === 'connect' && connect}{tab === 'brainstorm' && brainstorm}{tab === 'versions' && versions}{tab === 'notes' && notes}
          {footer}
        </>
      ))}

      <Sheet visible={stageSheet} onClose={() => setStageSheet(false)} label="Stage" testID="stage-sheet">
        <T v="h2" style={{ marginBottom: 8 }}>Stage of {title}</T>
        <View style={{ gap: 6 }}>
          {STAGES.filter((st) => st !== J?.stage).map((st) => (
            <Btn key={st} testID={`to-${st}`} label={`Move to ${stageWord(st)}`} onPress={async () => {
              setStageSheet(false);
              const w = await decide({ target: 'node', action: 'edit', id, after: { stage: st } });
              ui.snack(`${title} → ${stageWord(st)}`, async () => { await undo(w.indexes); d.reload(); });
              d.reload();
            }} />
          ))}
        </View>
      </Sheet>
      <Sheet visible={more} onClose={() => setMore(false)} label="More" testID="joke-more-sheet">
        <T v="h2" style={{ marginBottom: 8 }}>{title}</T>
        <View style={{ gap: 8 }}>
          <Btn label="Copy the text" onPress={async () => {
            setMore(false);
            try { await (globalThis as any).navigator?.clipboard?.writeText(hero?.text || ''); ui.snack('Copied'); } catch { ui.snack('Copy isn’t available here'); }
          }} />
          <Btn label="Change stage…" onPress={() => { setMore(false); setStageSheet(true); }} />
          {J?.stage !== 'retired' && <Btn testID="retire" v="danger" label="Retire" onPress={async () => {
            setMore(false);
            const w = await decide({ target: 'node', action: 'edit', id, after: { stage: 'retired' } });
            ui.snack(`${title} retired`, async () => { await undo(w.indexes); d.reload(); });
            d.reload();
          }} />}
        </View>
      </Sheet>
    </Screen>
  );
}

// ------------------------------------------------------------------ Connect
const ConnRow = memo(function ConnRow({ it, g, i, jokeId }: { it: ConnItem; g: ConnGroupKey; i: number; jokeId: string }) {
  const why = useWhy();
  const ui = useUI();
  const boards = useBoards();
  const isProven = proven(it);
  const t = it.title.split(' (')[0];
  const openWhy = () => why.why({
    title: GROUP_TITLE[g], who: it.source === 'his' ? 'yours' : isProven ? 'measured' : 'ai', claim: it.line ? `“${it.line}”` : t, reason: it.why, confidence: it.score,
    whoLine: isProven ? 'Done back-to-back on stage.' : it.source === 'his' ? 'You made this connection.' : `The AI suggested this (${(it.basis || []).join(', ')}).`,
    evidence: (it.evidence || []).map((e) => ({ ...e, jokeId: e.node.startsWith('unit:') ? (it.unit_id === e.node ? it.joke_id : jokeId) : undefined })),
    right: it.edge && it.source !== 'his' ? async () => {
      const w = await decide({ target: 'edge', id: it.edge, action: 'confirm' });
      ui.snack(`${t}: yours now`, async () => { await undo(w.indexes); });
    } : undefined,
    not: it.edge && it.source !== 'his' ? async () => {
      const w = await decide({ target: 'edge', id: it.edge, action: 'reject' });
      ui.snack(`${t}: dismissed`, async () => { await undo(w.indexes); });
    } : undefined,
    testID: `why-${g}-${i}`,
  });
  return (
    <View testID={`conn-${g}-${i}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 8, borderTopWidth: 1, borderTopColor: C.border }}>
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <Pressable accessibilityRole="button" onPress={() => router.push(`/joke/${encodeURIComponent(it.joke_id)}` as any)} style={{ flexShrink: 1, minHeight: 28, justifyContent: 'center' }}>
            <Text maxFontSizeMultiplier={1.4} numberOfLines={g === 'tags' ? 2 : 1} style={{ fontFamily: g === 'tags' ? F.body : F.semi, fontSize: 15, color: C.text }}>{g === 'tags' && it.line ? `“${it.line}”` : t}</Text>
          </Pressable>
          {isProven ? <Proven /> : it.source === 'his' ? <Yours /> : it.source === 'inferred' ? <AIBadge /> : null}
        </View>
        <WhyLine testID={`conn-why-${g}-${i}`} text={g === 'tags' ? `${t} · ${it.why}` : it.why} onPress={openWhy} />
      </View>
      <AddBtn testID={`conn-add-${g}-${i}`} onPress={() => boards.add(it.joke_id, t)} onLongPress={() => boards.pick(it.joke_id, t)} />
    </View>
  );
});

function ConnectTab({ id, title, conn, loading, error, wide, labels, onReload }: { id: string; title: string; conn: Connections | null; loading: boolean; error: string | null; wide: boolean; labels: EdgeView[]; onReload: () => void }) {
  const cap = wide ? 5 : 3;
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [longOpen, setLongOpen] = useState(false);
  const [connectSheet, setConnectSheet] = useState(false);
  const groups = useMemo(() => {
    const by = new Map<string, ConnGroup>((conn?.groups || []).map((g) => [g.key, g]));
    const long: { g: ConnGroupKey; it: ConnItem }[] = [];
    const out = GROUP_ORDER.map((k) => {
      const items = rankGroup(by.get(k)?.items || []);
      const keep = k === 'same_world' ? items : items.filter((it) => !longShot(it));
      if (k !== 'same_world') items.filter(longShot).forEach((it) => long.push({ g: k, it }));
      return { key: k, items: keep };
    }).filter((g) => g.items.length);
    return { out, long };
  }, [conn]);
  if (error && !conn) return <ErrorCard error={`${error} · connections load when you’re online`} onRetry={onReload} />;
  if (!conn) return <Empty text={loading ? 'Finding what goes with this…' : 'Connections load when you’re online.'} />;
  const worlds = (() => {
    const g = groups.out.find((x) => x.key === 'same_world');
    const names = new Set<string>();
    for (const it of g?.items || []) for (const n of [...(it.personas || []), ...(it.topics || []), ...(it.shared || [])]) if (typeof n === 'string') names.add(n);
    for (const l of labels) if ((l.other?.kind === 'persona' || l.other?.kind === 'topic') && l.state !== 'rejected') names.add(l.other.title);
    return [...names].slice(0, 8);
  })();
  return (
    <View testID="connect" style={{ gap: 14 }}>
      {!groups.out.length && !worlds.length && <Empty text="Nothing connects yet. Add a connection, or open Brainstorm." />}
      {groups.out.map((g) => {
        if (g.key === 'same_world') {
          return (
            <View key={g.key} testID="group-same_world" style={{ gap: 8 }}>
              <T v="eb">Same world</T>
              <Chips>{worlds.map((w) => <Chip key={w} testID={`world-${w}`} label={w} onPress={() => router.push(`/notebook?q=${encodeURIComponent(w)}` as any)} />)}</Chips>
              <WhyLine text={`${g.items.length} bits share a persona or topic with ${title}`} />
            </View>
          );
        }
        const shown = open[g.key] ? g.items : g.items.slice(0, cap);
        return (
          <View key={g.key} testID={`group-${g.key}`}>
            <T v="eb" style={{ marginBottom: 2 }}>{GROUP_TITLE[g.key]}</T>
            {shown.map((it, i) => <ConnRow key={`${it.id}-${i}`} it={it} g={g.key} i={i} jokeId={id} />)}
            {g.items.length > cap && !open[g.key] && <TextBtn testID={`show-more-${g.key}`} label={`Show ${g.items.length - cap} more`} onPress={() => setOpen((o) => ({ ...o, [g.key]: true }))} />}
          </View>
        );
      })}
      {!!groups.long.length && (
        <View testID="long-shots">
          <TextBtn testID="long-shots-toggle" color={C.mut} label={`${longOpen ? '▾' : '▸'} Long shots (${groups.long.length})`} onPress={() => setLongOpen((x) => !x)} />
          {longOpen && groups.long.map(({ g, it }, i) => <ConnRow key={`ls-${it.id}-${i}`} it={it} g={g} i={100 + i} jokeId={id} />)}
        </View>
      )}
      <Btn testID="connect-to" label="＋ Connect to…" v="add" onPress={() => setConnectSheet(true)} />
      <ConnectSheet visible={connectSheet} onClose={() => setConnectSheet(false)} jid={id} title={title} />
    </View>
  );
}

function ConnectSheet({ visible, onClose, jid, title }: { visible: boolean; onClose: () => void; jid: string; title: string }) {
  const ui = useUI();
  const [q, setQ] = useState('');
  const [pick, setPick] = useState<{ id: string; title: string } | null>(null);
  const [kind, setKind] = useState(KIND_FOR[0]);
  const [reason, setReason] = useState('');
  const res = useApi<{ items: any[] }>(q.trim().length >= 2 ? `/api/jokes?q=${encodeURIComponent(q.trim())}&limit=8` : null, { cache: false });
  useEffect(() => {
    if (visible) { setQ(''); setPick(null); setReason(''); setKind(KIND_FOR[0]); }
  }, [visible]);
  const input = { minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 } as const;
  return (
    <Sheet visible={visible} onClose={onClose} label="Connect to…" testID="connect-sheet">
      <T v="h2" style={{ marginBottom: 8 }}>Connect {title} to…</T>
      {!pick ? (
        <View style={{ gap: 8 }}>
          <TextInput testID="link-search" value={q} onChangeText={setQ} placeholder="Find a bit" placeholderTextColor={C.dim} style={input} autoFocus />
          <List>{(res.data?.items || []).filter((j) => j.id !== jid).map((j) => <ListRow key={j.id} testID={`pick-${j.id}`} title={j.title} onPress={() => setPick({ id: j.id, title: j.title })} />)}</List>
        </View>
      ) : (
        <View style={{ gap: 10 }}>
          <T v="semi">{pick.title}</T>
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
            {KIND_FOR.map((k) => <Chip key={k.key} testID={`kind-${k.key}`} label={k.label} on={kind.key === k.key} onPress={() => setKind(k)} />)}
          </View>
          <TextInput testID="link-reason" value={reason} onChangeText={setReason} placeholder="Why? e.g. both end on a callback" placeholderTextColor={C.dim} style={input} />
          <Btn testID="link-save" label="Connect" v="primary" disabled={!reason.trim()} onPress={async () => {
            onClose();
            try {
              const w = await decide({ target: 'edge', action: 'add', after: { src: jid, dst: pick.id, kind: kind.kind, ...(kind.kind === 'custom' ? { display_kind: 'contrast' } : {}) }, reason: reason.trim() });
              ui.snack(`Connected to ${pick.title.split(' (')[0]}`, async () => { await undo(w.indexes); });
            } catch (e: any) { ui.snack(`Couldn't connect: ${e.message}`); }
          }} />
        </View>
      )}
    </Sheet>
  );
}

// ------------------------------------------------------------------ Brainstorm
function BrainstormTab({ id, title, wsId, locked, conn }: { id: string; title: string; wsId: string | null; locked: boolean; conn: Connections | null }) {
  const ui = useUI();
  const [state, setState] = useState<{ status: string; pu?: PunchUp; msg?: string } | null>(null);
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());
  const give = async (fresh = false) => {
    if (!wsId) {
      setState({ status: 'unavailable', msg: 'Drafts run for bits in your Working set for now. Add this bit to the Working set, or wait for the general punch-up route.' });
      return;
    }
    setState({ status: 'asking' });
    try {
      const r: any = await creative.punchup(wsId, id, fresh);
      if (r.status === 'queued') setState({ status: 'queued', msg: 'Queued for your home worker. Drafts appear here when they’re ready.' });
      else setState({ status: r.status || 'ok', pu: r });
    } catch (e: any) {
      setState({ status: 'error', msg: `The AI provider didn’t answer (${e.message}).` });
    }
  };
  const useAsMine = async (text: string, i: number) => {
    try {
      const uid = `unit:draft-${id.replace(/^joke:/, '')}-${Date.now().toString(36)}`;
      const a = await decide({ target: 'node', action: 'add', after: { id: uid, type: 'unit', kind: 'tag', text, source: 'draft', date: new Date().toISOString().slice(0, 10) } });
      const b = await decide({ target: 'edge', action: 'add', after: { src: uid, dst: id, kind: 'variant_of' }, reason: 'Used an AI draft as mine (Brainstorm)' });
      setDismissed((x) => new Set(x).add(i));
      ui.snack('Saved as your version', async () => { await undo([...b.indexes, ...a.indexes]); });
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };
  const drafts = (state?.pu?.drafts || []).map((d, i) => ({ d, i })).filter(({ i }) => !dismissed.has(i));
  return (
    <View testID="brainstorm" style={{ gap: 12 }}>
      <T v="eb">Riff from your patterns</T>
      {!state && (
        <View style={{ gap: 6 }}>
          <Btn testID="give-ideas" label="Give me 3 ideas" v="add" onPress={() => give()} disabled={locked} />
          <T v="small">{locked ? 'This bit is locked: no drafts on locked lines.' : 'Uses Claude Opus 5.5 on your home worker · sees the text of this bit only.'}</T>
        </View>
      )}
      {state?.status === 'asking' && <T v="mut">Asking…</T>}
      {state?.msg && <T v="mut" testID="brainstorm-state">{state.msg}</T>}
      {state?.pu?.status === 'locked' && <T v="mut">Locked: no drafts on locked lines.</T>}
      {drafts.map(({ d, i }) => (
        <View key={i} testID={`draft-${i}`} style={[s.draft, { gap: 8 }]}>
          <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
            <View style={{ paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.addBg, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft</Text></View>
            <T v="small" style={{ flex: 1 }}>{d.move} · not your words yet</T>
          </View>
          <Text maxFontSizeMultiplier={1.5} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.amber }}>{d.alt}</Text>
          <T v="small">modelled on: “{d.target_line.slice(0, 80)}{d.target_line.length > 80 ? '…' : ''}”</T>
          <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
            <Btn testID={`draft-use-${i}`} small label="Use as mine" onPress={() => useAsMine(d.alt, i)} />
            <Btn small label="Another" v="ghost" onPress={() => give(true)} />
            <Btn testID={`draft-dismiss-${i}`} small label="Dismiss" v="ghost" onPress={() => setDismissed((x) => new Set(x).add(i))} />
          </View>
        </View>
      ))}
      {!!state?.pu?.provider && <T v="small">drafted by {state.pu.model || state.pu.provider}</T>}
      {!!conn?.land_harder?.length && (
        <View style={{ gap: 8 }} testID="land-harder">
          <T v="eb">What would make it land harder</T>
          {conn.land_harder.map((l, i) => (
            <View key={i} style={[s.inner, { gap: 4 }]}>
              <T>{l.text}</T>
              <T v="small">{l.basis}</T>
            </View>
          ))}
        </View>
      )}
    </View>
  );
}

// ------------------------------------------------------------------ Versions
const firstNewSentence = (text: string, prev: string) => {
  const sents = text.split(/(?<=[.!?])\s+/).map((x) => x.trim()).filter(Boolean);
  const p = prev.toLowerCase();
  return sents.find((x) => !p.includes(x.toLowerCase().slice(0, 40))) || sents[sents.length - 1] || text;
};
function VersionsTab({ d, nights, bestSet, unit, onChanged }: { d: any; nights: any[]; bestSet: string | null; unit: string | null; onChanged: () => void }) {
  const ui = useUI();
  const [openYears, setOpenYears] = useState<Record<string, boolean>>({});
  const [fullU, setFullU] = useState<Record<string, boolean>>({});
  const [notThis, setNotThis] = useState<any | null>(null);
  const units: any[] = useMemo(() => (d?.versions || []).flatMap((v: any) => v.units.map((u: any) => ({ ...u, date: u.date || v.date }))), [d]);
  const entries = useMemo(() => {
    const written = units.map((u, k) => ({ kind: 'doc' as const, key: u.id, date: u.date || '', u, line: firstNewSentence(u.text || '', k ? units[k - 1].text || '' : '') }));
    const stage = nights.map((n) => ({ kind: 'stage' as const, key: n.set, date: n.date || '', n }));
    return [...written, ...stage].sort((a, b) => b.date.localeCompare(a.date));
  }, [units, nights]);
  const byYear = useMemo(() => {
    const m = new Map<string, typeof entries>();
    for (const e of entries) {
      const y = e.date.slice(0, 4) || '—';
      if (!m.has(y)) m.set(y, []);
      m.get(y)!.push(e);
    }
    return [...m.entries()];
  }, [entries]);
  const unitYear = unit ? (units.find((u) => u.id === unit)?.date || '').slice(0, 4) : null;
  useEffect(() => {
    if (unitYear) setOpenYears((o) => ({ ...o, [unitYear]: true }));
    if (unit) setFullU((f) => ({ ...f, [unit]: true }));
  }, [unitYear, unit]);
  if (!d) return null;
  return (
    <View testID="versions" style={{ gap: 6 }}>
      {byYear.map(([y, es], yi) => {
        const isOpen = openYears[y] ?? yi === 0;
        const nDocs = es.filter((e) => e.kind === 'doc').length;
        const nStage = es.length - nDocs;
        return (
          <View key={y} style={{ gap: 6 }}>
            <Pressable testID={`year-${y}`} accessibilityRole="button" accessibilityState={{ expanded: isOpen }} onPress={() => setOpenYears((o) => ({ ...o, [y]: !isOpen }))}
              style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44, gap: 8, borderTopWidth: 1, borderTopColor: C.border }}>
              <T v="semi" style={{ flex: 1 }}>{y}</T>
              <T v="small">{[nDocs ? `${nDocs} ${nDocs === 1 ? 'draft' : 'drafts'}` : '', nStage ? `${nStage} on stage` : ''].filter(Boolean).join(' · ')} {isOpen ? '▾' : '▸'}</T>
            </Pressable>
            {isOpen && es.map((e) => e.kind === 'doc' ? (
              <Pressable key={e.key} testID={`unit-${e.u.id}`} onLongPress={() => setNotThis(e.u)} accessibilityHint="Long-press: not this joke"
                style={[s.inner, { gap: 4 }, unit === e.u.id && { borderColor: C.amber, backgroundColor: C.amberSoft }]}>
                <View style={{ flexDirection: 'row', gap: 6, alignItems: 'center' }}>
                  <Icon name="desk" size={14} color={C.mut} />
                  <T v="small" style={{ flex: 1 }} numberOfLines={1}>{e.u.doc_title || 'doc'} · {fmtShortDate(e.date)} · {e.u.kind}</T>
                </View>
                <T style={{ fontSize: 15 }}>{fullU[e.u.id] ? e.u.text : e.line}</T>
                <TextBtn label={fullU[e.u.id] ? 'Less' : 'Show full text'} color={C.mut} onPress={() => setFullU((f) => ({ ...f, [e.u.id]: !f[e.u.id] }))} />
              </Pressable>
            ) : (
              <View key={e.key} testID={`stage-${e.n.set}`} style={[s.inner, { gap: 4 }]}>
                <View style={{ flexDirection: 'row', gap: 6, alignItems: 'center' }}>
                  <Icon name="mic" size={14} color={C.mut} />
                  <T v="small" style={{ flex: 1 }}>{e.n.label} · {fmt1(e.n.laugh_s_per_min)} laugh-s/min{e.n.excluded ? ' · audio unreliable' : ''}</T>
                  {bestSet === e.n.set && <View style={{ paddingHorizontal: 6, borderRadius: 4, backgroundColor: C.amberSoft }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>best</Text></View>}
                </View>
                {!!e.n.perf.punch?.text && <T style={{ fontSize: 15 }}>“{e.n.perf.punch.text}”</T>}
                {(e.n.perf.audio || youtubeSource(e.n.perf.reference)) && <Btn small v="ghost" label="▶ Hear it" onPress={() => playSegment(e.n.perf.audio || youtubeSource(e.n.perf.reference)!, e.n.perf.play.start_ms, e.n.perf.play.end_ms, `v-${e.n.set}`).catch((er) => ui.snack(er.message))} />}
              </View>
            ))}
          </View>
        );
      })}
      <Sheet visible={!!notThis} onClose={() => setNotThis(null)} label="Not this joke" testID="not-this-sheet">
        <T v="h2">Not this joke?</T>
        <T v="mut" style={{ marginVertical: 8 }}>{notThis?.doc_title} · {notThis?.date}</T>
        <Btn testID="not-this-confirm" v="danger" label="Not this joke" onPress={async () => {
          const e = notThis?.edge;
          setNotThis(null);
          if (!e?.id) return;
          const w = await decide({ target: 'edge', id: e.id, action: 'reject' });
          ui.snack('Removed from this joke', async () => { await undo(w.indexes); onChanged(); });
          onChanged();
        }} />
      </Sheet>
    </View>
  );
}

// ------------------------------------------------------------------ Notes
function NotesTab({ J, conn }: { J: any; conn: Connections | null }) {
  const note = J?.quality?.notes;
  return (
    <View testID="joke-notes" style={{ gap: 10 }}>
      {!!note && <View style={s.inner}><View style={s.row}><T v="small" style={{ flex: 1 }}>Note from the catalog pass</T><AIBadge /></View><T>{note}</T></View>}
      <T v="mut">Voice notes that mention this bit show up here once a note is transcribed.</T>
      {!!conn?.notes?.length && <T v="small">{conn.notes[0]}</T>}
    </View>
  );
}
