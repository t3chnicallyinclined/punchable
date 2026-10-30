// Notebook (SIMPLE.md; was Desk, UX_SPEC 4.2): search first. "Search your bits, or ask" · Bits | Notes | Shows ·
// filters behind "Filter" · count + sort · virtualized rows. A question turns into a conversation: Ask (/ask?q=…).
// Shows (sets, laughs, the map, import, record) live here, one tap down. The Pipeline view is gone (SIMPLE.md).
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FlatList, Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { JokeSummary, decide, get, undo, useApi } from '@/lib/api';
import { strengthDots } from '@/rules/laughs';
import { StrengthDots } from '@/lib/metrics';
import { useBoards } from '@/lib/boards';
import { C, F, STAGE, STAGES, WIDE, stageInfo, stageWord } from '@/lib/theme';
import { AddBtn, Avatar, Btn, Chip, Chips, Empty, ErrorCard, Icon, List, ListRow, Seg, Sheet, StageWord, T, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { MergeGroups } from '@/lib/MergeHint';
import { isQuestion } from '@/lib/say';
import { useSets } from '@/lib/sets';
import { fmtShortDate } from '@/rules/format';
import { WhyLine } from '@/lib/why';
import { VoiceMemos } from '@/lib/memos';

type View_ = 'jokes' | 'notes' | 'shows';
type Sort = 'performed' | 'strongest' | 'written' | 'untouched';
type Label = { id: string; name: string; kind: string; definition?: string };
const SORTS: [Sort, string][] = [['performed', 'Recently performed'], ['strongest', 'Strongest'], ['written', 'Recently written'], ['untouched', 'Oldest untouched']];
const FACETS: { key: string; label: string; kinds?: string[] }[] = [
  { key: 'stage', label: 'Stage' }, { key: 'topic', label: 'Topic', kinds: ['topic'] }, { key: 'persona', label: 'Persona', kinds: ['persona'] },
  { key: 'craft', label: 'Craft', kinds: ['structure', 'device', 'form'] }, { key: 'attitude', label: 'Attitude', kinds: ['attitude'] },
  { key: 'onstage', label: 'On stage' }, { key: 'year', label: 'Year' },
];
const ROW_H = 84;

/** Short title: the API's (AI-written) short title, else the catalog title cut at the first parenthesis, ≤ 28 chars. */
export function shortTitle(j: { title: string; short_title?: string }) {
  if (j.short_title) return j.short_title;
  const t = j.title.split(' (')[0].trim();
  return t.length > 28 ? `${t.slice(0, 27).trimEnd()}…` : t;
}

const Row = memo(function Row({ j, onAdd, onPick, onStage }: { j: JokeSummary; onAdd: (j: JokeSummary) => void; onPick: (j: JokeSummary) => void; onStage: (j: JokeSummary) => void }) {
  const performed = (j.sets || 0) > 0;
  const nights = j.nights ?? j.sets ?? 0;
  const dots = strengthDots(j.avg_vs_room ?? (performed ? ((j.ai_hit_sets ?? j.hit_sets ?? 0) / Math.max(1, j.sets)) * 1.6 : null), performed);
  const year = (j.last_performed || j.last_seen || '').slice(0, 4);
  return (
    <View testID={`joke-${j.id}`} style={{ height: ROW_H, flexDirection: 'row', alignItems: 'center', gap: 8, borderBottomWidth: 1, borderBottomColor: C.border }}>
      <Pressable accessibilityRole="button" accessibilityLabel={`${j.title}, ${stageWord(j.stage)}`} onPress={() => router.push(`/joke/${encodeURIComponent(j.id)}` as any)}
        style={{ flex: 1, minWidth: 0, height: '100%', justifyContent: 'center', gap: 2 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <Text maxFontSizeMultiplier={1.3} numberOfLines={1} style={{ flex: 1, fontFamily: F.semi, fontSize: 16, color: C.text }}>{shortTitle(j)}</Text>
        </View>
        {!!j.punch && <Text maxFontSizeMultiplier={1.3} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 14, color: C.text2 }}>“…{j.punch}”</Text>}
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <StrengthDots n={dots} />
          <Text maxFontSizeMultiplier={1.3} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 12, color: C.mut }}>
            {performed ? `${nights} ${nights === 1 ? 'night' : 'nights'}` : 'not on tape'}{year ? ` · ${year}` : ''}
          </Text>
        </View>
      </Pressable>
      <Pressable testID={`stage-${j.id}`} accessibilityRole="button" accessibilityLabel={`Stage ${stageWord(j.stage)}. Change`} onPress={() => onStage(j)} style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: 4 }}>
        <StageWord stage={j.stage} ai={!!j.stage_ai && j.who !== 'confirmed' && j.who !== 'edited'} />
      </Pressable>
      <AddBtn testID={`add-${j.id}`} onPress={() => onAdd(j)} onLongPress={() => onPick(j)} />
    </View>
  );
});

/** The whole catalogue: the API pages at 500, so page 1 tells the total and the rest load in parallel. */
const PAGE = 500;
async function fetchAll(path: string) {
  const q = path.includes('?') ? path.split('?')[1].replace(/(^|&)all=1/, '') : '';
  const url = (off: number) => `/api/jokes?${q ? q + '&' : ''}offset=${off}&limit=${PAGE}`;
  const first = await get<{ items: JokeSummary[]; total: number }>(url(0));
  const rest = await Promise.all(Array.from({ length: Math.max(0, Math.ceil(first.total / PAGE) - 1) }, (_, i) => get<{ items: JokeSummary[] }>(url((i + 1) * PAGE))));
  return { total: first.total, items: first.items.concat(...rest.map((r) => r.items)) };
}

function useLabelIds(ids: string[]) {
  const [sets, setSets] = useState<Record<string, Set<string>>>({});
  useEffect(() => {
    let alive = true;
    for (const id of ids) {
      if (sets[id]) continue;
      fetchAll(`/api/jokes?label=${encodeURIComponent(id)}`).then((r) => {
        if (alive) setSets((x) => ({ ...x, [id]: new Set(r.items.map((i) => i.id)) }));
      }).catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [ids.join('|')]); // eslint-disable-line react-hooks/exhaustive-deps
  return sets;
}

export default function Desk() {
  const ui = useUI();
  const boards = useBoards();
  const wide = useWide();
  const q0 = useLocalSearchParams<{ view?: string; q?: string; who?: string; sort?: string }>();
  const [view, setView] = useState<View_>((['jokes', 'notes', 'shows'].includes(String(q0.view)) ? q0.view : 'jokes') as View_);
  const [filters, setFilters] = useState(false);
  const [qText, setQText] = useState(q0.q || '');
  const [q, setQ] = useState(q0.q || '');
  const [sort, setSort] = useState<Sort>((q0.sort as Sort) || 'performed');
  const [stage, setStage] = useState<string | null>(null);
  const [labels, setLabels] = useState<Label[]>([]);
  const [onstage, setOnstage] = useState<'performed' | 'never' | null>(null);
  const [year, setYear] = useState<number | null>(null);
  const [facet, setFacet] = useState<string | null>(null);
  const [sortSheet, setSortSheet] = useState(false);
  const [stageSheet, setStageSheet] = useState<JokeSummary | null>(null);
  const [facetQ, setFacetQ] = useState('');

  useEffect(() => {
    const t = setTimeout(() => setQ(qText.trim()), 250);
    return () => clearTimeout(t);
  }, [qText]);

  const all = useApi<{ items: JokeSummary[]; total: number }>('/api/jokes?all=1', { fetcher: fetchAll });
  const search = useApi<{ items: JokeSummary[]; total: number }>(q && view === 'jokes' ? `/api/jokes?q=${encodeURIComponent(q)}&mode=hybrid&limit=${PAGE}` : null, { cache: false });
  const labelList = useApi<{ labels: Label[] }>('/api/labels');
  const labelIds = useLabelIds(labels.map((l) => l.id));

  const base = q ? search.data?.items : all.data?.items;
  const items = useMemo(() => {
    let xs = base || [];
    if (q0.who) xs = xs.filter((j) => j.who === q0.who);
    if (stage) xs = xs.filter((j) => j.stage === stage);
    if (onstage) xs = xs.filter((j) => (onstage === 'performed') === ((j.sets || 0) > 0));
    if (year) xs = xs.filter((j) => Number((j.first_seen || '').slice(0, 4)) === year);
    for (const l of labels) {
      const ids = labelIds[l.id];
      if (ids) xs = xs.filter((j) => ids.has(j.id));
    }
    if (q) return xs; // search keeps the API's relevance order
    const perfKey = (j: JokeSummary) => j.last_performed || ((j.sets || 0) > 0 ? `p${j.last_seen || ''}` : '');
    const strength = (j: JokeSummary) => j.avg_vs_room ?? ((j.sets || 0) > 0 ? (j.ai_hit_sets ?? j.hit_sets ?? 0) / j.sets + j.sets / 100 : -1);
    const out = [...xs];
    if (sort === 'performed') out.sort((a, b) => perfKey(b).localeCompare(perfKey(a)) || (b.last_seen || '').localeCompare(a.last_seen || ''));
    if (sort === 'strongest') out.sort((a, b) => strength(b) - strength(a));
    if (sort === 'written') out.sort((a, b) => (b.last_seen || '').localeCompare(a.last_seen || ''));
    if (sort === 'untouched') out.sort((a, b) => (a.last_seen || '9').localeCompare(b.last_seen || '9'));
    return out;
  }, [base, stage, onstage, year, labels, labelIds, sort, q, q0.who]);

  const onAdd = useCallback((j: JokeSummary) => boards.add(j.id, shortTitle(j)), [boards]);
  const onPick = useCallback((j: JokeSummary) => boards.pick(j.id, shortTitle(j)), [boards]);
  const onStage = useCallback((j: JokeSummary) => setStageSheet(j), []);

  /** A question opens Ask: a conversation with his notebook, answered only from his own material (src/lib/ask.tsx). */
  const runAsk = (question: string) => router.push({ pathname: '/ask', params: { q: question } } as any);
  const submit = () => {
    const t = qText.trim();
    setQ(t);
    if (t && isQuestion(t)) runAsk(t);
  };

  const changeStage = async (j: JokeSummary, st: string) => {
    setStageSheet(null);
    try {
      const w = await decide({ target: 'node', action: 'edit', id: j.id, after: { stage: st } });
      ui.snack(`${shortTitle(j)} → ${stageWord(st)}`, async () => { await undo(w.indexes); all.reload(); });
      all.reload();
    } catch (e: any) {
      ui.snack(`Couldn't change the stage: ${e.message}`);
    }
  };
  const confirmStage = async (j: JokeSummary) => {
    setStageSheet(null);
    const w = await decide({ target: 'node', action: 'confirm', id: j.id, after: { status: 'confirmed' } });
    ui.snack(`${stageWord(j.stage)} is right`, async () => { await undo(w.indexes); all.reload(); });
    all.reload();
  };

  const total = q ? search.data?.total : all.data?.total;
  const filtered = !!(stage || labels.length || onstage || year || q0.who);
  const facetChips = (
    <Chips testID="facets">
      {stage && <Chip testID="facet-on-stage" label={stageWord(stage)} on onRemove={() => setStage(null)} />}
      {labels.map((l) => <Chip key={l.id} testID={`facet-on-${l.id}`} label={l.name} on onRemove={() => setLabels((x) => x.filter((y) => y.id !== l.id))} />)}
      {onstage && <Chip testID="facet-on-onstage" label={onstage === 'performed' ? 'Performed' : 'Never performed'} on onRemove={() => setOnstage(null)} />}
      {year && <Chip testID="facet-on-year" label={String(year)} on onRemove={() => setYear(null)} />}
      {FACETS.map((f) => <Chip key={f.key} testID={`facet-${f.key}`} label={`${f.label} ▾`} onPress={() => { setFacetQ(''); setFacet(f.key); }} />)}
    </Chips>
  );

  const searchBox = (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, borderWidth: 1, borderColor: C.border, borderRadius: 12, paddingHorizontal: 12, backgroundColor: C.surface }}>
      <Icon name="search" size={18} color={C.mut} />
      <TextInput testID="desk-search" value={qText} onChangeText={setQText} onSubmitEditing={submit} returnKeyType="search"
        placeholder="Search your bits, or ask" placeholderTextColor={C.dim} accessibilityLabel="Search your bits, or ask"
        style={{ flex: 1, minHeight: 44, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      {!!qText && <Pressable testID="desk-clear" accessibilityRole="button" accessibilityLabel="Clear" onPress={() => { setQText(''); setQ(''); }} style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.mut, fontSize: 18 }}>×</Text></Pressable>}
    </View>
  );

  const askBlock = q && view === 'jokes' ? (
    <Chip testID="ask-instead" label={`Ask instead: “${q.length > 30 ? q.slice(0, 29) + '…' : q}”`} onPress={() => runAsk(q)} />
  ) : null;

  const header = (
    <View style={{ gap: 12, paddingTop: 12, paddingBottom: 8, paddingHorizontal: wide ? 0 : 16 }}>
      {searchBox}
      <Seg testID="desk-view" items={[['jokes', 'Bits'], ['notes', 'Notes'], ['shows', 'Shows']]} value={view} onChange={setView} />
      {view === 'jokes' && (filters || filtered ? facetChips : (
        <View style={{ flexDirection: 'row' }}><Chip testID="filter-toggle" label="Filter ▾" onPress={() => setFilters(true)} /></View>
      ))}
      {view === 'jokes' && !q && <MergeGroups />}
      {view === 'jokes' && askBlock}
      {view === 'jokes' && (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <T v="mut" testID="result-count" style={{ flex: 1 }}>{total == null ? 'Loading…' : `${(filtered ? items.length : total).toLocaleString('en-US')} ${q ? 'matches' : 'bits'}`}</T>
          {!q && <Pressable testID="sort" accessibilityRole="button" onPress={() => setSortSheet(true)} style={{ minHeight: 44, justifyContent: 'center' }}><T v="mut" style={{ color: C.text2 }}>{SORTS.find((x) => x[0] === sort)![1]} ▾</T></Pressable>}
        </View>
      )}
    </View>
  );

  const facetDef = FACETS.find((f) => f.key === facet);
  const opts = useMemo(() => {
    if (!facetDef?.kinds) return [];
    return (labelList.data?.labels || []).filter((l) => facetDef.kinds!.includes(l.kind) && l.name.toLowerCase().includes(facetQ.toLowerCase())).slice(0, 60);
  }, [facetDef, labelList.data, facetQ]);
  const years = useMemo(() => [...new Set((all.data?.items || []).map((j) => Number((j.first_seen || '').slice(0, 4))).filter(Boolean))].sort((a, b) => b - a), [all.data]);

  return (
    <Screen title="Notebook" scroll={false} testID="desk" right={<Avatar />}>
      {view === 'jokes' ? (
        <FlatList
          testID="joke-list"
          contentInsetAdjustmentBehavior="automatic"
          data={items}
          keyExtractor={(j) => j.id}
          renderItem={({ item }) => <View style={{ paddingHorizontal: wide ? 0 : 16 }}><Row j={item} onAdd={onAdd} onPick={onPick} onStage={onStage} /></View>}
          getItemLayout={(_, i) => ({ length: ROW_H, offset: ROW_H * i, index: i })}
          initialNumToRender={14}
          maxToRenderPerBatch={16}
          windowSize={7}
          removeClippedSubviews
          ListHeaderComponent={header}
          ListEmptyComponent={
            (all.error && !all.data) ? <View style={{ padding: 16 }}><ErrorCard error={all.error} onRetry={all.reload} /></View>
              : (q && search.data) ? <Empty text={`Nothing matches “${q}”. Ask instead?`}><Btn small label="Ask" onPress={() => runAsk(q)} /></Empty>
                : (base ? <Empty text="Nothing matches these filters." /> : <Empty text="Loading your bits…" />)
          }
          contentContainerStyle={{ paddingBottom: 40, width: '100%', maxWidth: wide ? 760 : undefined, alignSelf: 'center' }}
          style={{ flex: 1 }}
          keyboardShouldPersistTaps="handled"
        />
      ) : (
        <FlatList data={[0]} keyExtractor={() => 'x'} contentInsetAdjustmentBehavior="automatic" ListHeaderComponent={header} contentContainerStyle={{ paddingBottom: 120, paddingHorizontal: wide ? 32 : 16, width: '100%', maxWidth: wide ? 1120 : 720, alignSelf: 'center' }}
          renderItem={() => (view === 'notes' ? <NotesList /> : <ShowsList />)} />
      )}

      <Sheet visible={!!facet} onClose={() => setFacet(null)} label={facetDef?.label || 'Filter'} testID="facet-sheet">
        <T v="h2" style={{ marginBottom: 8 }}>{facetDef?.label}</T>
        {facet === 'stage' && <View style={{ gap: 6 }}>{STAGES.map((st) => <ListRow key={st} testID={`pick-stage-${st}`} title={stageWord(st)} sub={STAGE[st].help} onPress={() => { setStage(st); setFacet(null); }} />)}</View>}
        {facet === 'onstage' && <View style={{ gap: 6 }}>
          <ListRow testID="pick-performed" title="Performed" sub="On tape at least once" onPress={() => { setOnstage('performed'); setFacet(null); }} />
          <ListRow testID="pick-never" title="Never performed" sub="Written only" onPress={() => { setOnstage('never'); setFacet(null); }} />
        </View>}
        {facet === 'year' && <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>{years.map((y) => <Chip key={y} testID={`pick-year-${y}`} label={String(y)} onPress={() => { setYear(y); setFacet(null); }} />)}</View>}
        {facetDef?.kinds && (
          <View style={{ gap: 6 }}>
            {(facetDef.kinds.includes('topic')) && <TextInput testID="facet-search" value={facetQ} onChangeText={setFacetQ} placeholder="Find a topic" placeholderTextColor={C.dim}
              style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />}
            {facetDef.key === 'craft' && <T v="small">Structures, devices and forms, as named in your joke science.</T>}
            <List>{opts.map((l) => <ListRow key={l.id} testID={`pick-label-${l.id}`} title={l.name} sub={l.kind} onPress={() => { setLabels((x) => (x.some((y) => y.id === l.id) ? x : [...x, l])); setFacet(null); }} />)}</List>
          </View>
        )}
        <T v="small" style={{ marginTop: 10 }}>A blue dot before a stage means the AI set it and you haven’t touched it.</T>
      </Sheet>

      <Sheet visible={sortSheet} onClose={() => setSortSheet(false)} label="Sort" testID="sort-sheet">
        <T v="h2" style={{ marginBottom: 8 }}>Sort</T>
        <List>{SORTS.map(([k, l]) => <ListRow key={k} testID={`sort-${k}`} title={l} right={sort === k ? <Text style={{ color: C.amber }}>●</Text> : undefined} onPress={() => { setSort(k); setSortSheet(false); }} chevron={false} />)}</List>
      </Sheet>

      <Sheet visible={!!stageSheet} onClose={() => setStageSheet(null)} label="Stage" testID="stage-sheet">
        {stageSheet && <>
          <T v="eb">Stage</T>
          <T v="h2" style={{ marginBottom: 4 }}>{shortTitle(stageSheet)}: {stageWord(stageSheet.stage)}</T>
          {stageSheet.stage_ai && <WhyLine text={`The AI set it${stageSheet.stage_confidence ? ` (${Math.round(stageSheet.stage_confidence * 100)}% sure)` : ''}. Open the bit for its reasons.`} onPress={() => { setStageSheet(null); router.push(`/joke/${encodeURIComponent(stageSheet.id)}` as any); }} />}
          <View style={{ gap: 6, marginTop: 8 }}>
            {stageSheet.stage_ai && <Btn testID="stage-right" label="This is right" v="primary" onPress={() => confirmStage(stageSheet)} />}
            {STAGES.filter((st) => st !== stageSheet.stage).map((st) => <Btn key={st} testID={`to-${st}`} label={`Move to ${stageWord(st)}`} onPress={() => changeStage(stageSheet, st)} />)}
          </View>
        </>}
      </Sheet>
    </Screen>
  );
}

function NotesList() {
  const n = useApi<{ items: any[] }>('/api/notes');
  return (
    <View style={{ gap: 12 }} testID="desk-notes">
      <Btn testID="desk-talk" label="Talk it out" v="primary" onPress={() => router.push({ pathname: '/notes', params: { talk: String(Date.now()) } } as any)} />
      <VoiceMemos />
      {n.data && !n.data.items.length && <Empty text="Talk out a premise. Funny lines get pulled out and filed." />}
      <List>
        {(n.data?.items || []).map((it) => (
          <ListRow key={it.file} title={it.title || 'Voice note'} sub={`${(it.received_at || '').slice(0, 10)}${it.duration_s ? ` · ${Math.round(it.duration_s)} s` : ''} · ${it.status === 'queued' ? 'queued for processing' : it.status}`} onPress={() => router.push('/notes' as any)} />
        ))}
      </List>
    </View>
  );
}

/** Shows, one tap down: the dated list, and the doors to the map, import, record and the after-a-show page. */
function ShowsList() {
  const sets = useSets();
  return (
    <View style={{ gap: 12 }} testID="desk-shows">
      <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
        <Btn testID="shows-import" small v="primary" label="Import sets" onPress={() => router.push('/import' as any)} />
        <Btn testID="shows-record" small label="Record a set" onPress={() => router.push('/record' as any)} />
        <Btn testID="shows-map" small label="Map" onPress={() => router.push('/shows?view=map' as any)} />
        <Btn testID="shows-after" small v="ghost" label="After a show ›" onPress={() => router.push('/today' as any)} />
      </View>
      {!sets.list.length && <Empty text={sets.loading ? 'Loading your shows…' : 'Import a set to start your show history.'} />}
      <List>
        {sets.list.map((x) => (
          <ListRow key={x.id} testID={`nb-show-${x.id}`} title={x.venue.name.replace(/\?$/, '')} sub={`${fmtShortDate(x.show.date)}${x.best ? ` · best: ${x.best.title.split(' (')[0]}` : ''}`}
            onPress={() => router.push(`/set/${encodeURIComponent(x.id)}` as any)} />
        ))}
      </List>
      <ListRow testID="shows-all" title="All shows" sub="Laughs per set, the map, what works in which room" onPress={() => router.push('/shows' as any)} />
    </View>
  );
}
